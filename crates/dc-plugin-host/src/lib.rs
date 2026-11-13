//! QuickJS sandbox: no std/os modules, module loader, fetch, filesystem or
//! clipboard bindings. Plugins only transform explicitly submitted text.
use dc_core::plugin::PluginManifest;
use rquickjs::{Context, Function, Runtime};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("Plugin execution failed or exceeded its limits")]
    Execution,
    #[error("Plugin requires an explicit outgoing-transform grant")]
    Permission,
}
pub struct ScriptPlugin {
    pub manifest: PluginManifest,
    runtime: Runtime,
    context: Context,
    deadline: Arc<Mutex<Instant>>,
}
impl ScriptPlugin {
    pub fn load(
        manifest: PluginManifest,
        source: &str,
        granted: &[String],
    ) -> Result<Self, PluginError> {
        manifest.validate().map_err(PluginError::Invalid)?;
        if !manifest
            .capabilities
            .iter()
            .any(|c| c == "outgoing-transform")
            || !granted.iter().any(|c| c == "outgoing-transform")
        {
            return Err(PluginError::Permission);
        }
        if source.len() > 256 * 1024 {
            return Err(PluginError::Invalid("Plugin source exceeds 256 KiB"));
        }
        let runtime = Runtime::new().map_err(|_| PluginError::Execution)?;
        runtime.set_memory_limit(16 * 1024 * 1024);
        runtime.set_max_stack_size(256 * 1024);
        let deadline = Arc::new(Mutex::new(Instant::now() + Duration::from_millis(50)));
        let interrupt = deadline.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            interrupt
                .lock()
                .map(|deadline| Instant::now() >= *deadline)
                .unwrap_or(true)
        })));
        let context = Context::full(&runtime).map_err(|_| PluginError::Execution)?;
        context.with(|ctx| {
            ctx.eval::<(), _>(source)
                .map_err(|_| PluginError::Execution)?;
            ctx.globals()
                .get::<_, Function>("transformOutgoing")
                .map_err(|_| PluginError::Invalid("Define function transformOutgoing(text)"))?;
            Ok(())
        })?;
        Ok(Self {
            manifest,
            runtime,
            context,
            deadline,
        })
    }
    pub fn transform(&self, text: &str) -> Result<String, PluginError> {
        if text.len() > 64 * 1024 {
            return Err(PluginError::Invalid("Input exceeds 64 KiB"));
        }
        *self.deadline.lock().map_err(|_| PluginError::Execution)? =
            Instant::now() + Duration::from_millis(20);
        let result = self.context.with(|ctx| {
            let function: Function = ctx
                .globals()
                .get("transformOutgoing")
                .map_err(|_| PluginError::Execution)?;
            function
                .call::<_, String>((text,))
                .map_err(|_| PluginError::Execution)
        });
        self.runtime.run_gc();
        let text = result?;
        if text.len() > 64 * 1024 {
            return Err(PluginError::Invalid("Plugin output exceeds 64 KiB"));
        }
        Ok(text)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> PluginManifest {
        PluginManifest {
            id: "demo".into(),
            name: "Demo".into(),
            api_version: 1,
            capabilities: vec!["outgoing-transform".into()],
        }
    }
    #[test]
    fn explicit_grant_is_required_and_transform_receives_only_text() {
        assert!(matches!(
            ScriptPlugin::load(manifest(), "function transformOutgoing(t){return t}", &[]),
            Err(PluginError::Permission)
        ));
        let p=ScriptPlugin::load(manifest(),"function transformOutgoing(t){if(typeof fetch!=='undefined'||typeof std!=='undefined'||typeof os!=='undefined')throw Error('IO');return t+' ✦'}",&["outgoing-transform".into()]).unwrap();
        assert_eq!(p.transform("hello").unwrap(), "hello ✦");
    }
    #[test]
    fn infinite_loops_are_interrupted() {
        let p = ScriptPlugin::load(
            manifest(),
            "function transformOutgoing(t){while(true){}}",
            &["outgoing-transform".into()],
        )
        .unwrap();
        assert!(p.transform("hello").is_err());
    }
}
