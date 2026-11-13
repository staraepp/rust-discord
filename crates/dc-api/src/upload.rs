//! Multipart messages from files explicitly selected by the owner.
use crate::{ApiClient, ApiError, HttpTransport, Route};
use dc_protocol::{
    Snowflake,
    model::{CreateMessage, Message},
    time::Clock,
};

/// File bytes intentionally have no Debug implementation.
pub struct UploadFile {
    pub filename: String,
    pub bytes: Vec<u8>,
}
impl<H: HttpTransport, C: Clock> ApiClient<H, C> {
    // ASSUMPTION: the documented multipart message route is accepted for
    // user accounts. Verify live; cloud pre-upload is not implemented yet.
    pub async fn create_message_with_files(
        &self,
        channel: Snowflake,
        message: &CreateMessage,
        files: &[UploadFile],
    ) -> Result<Message, ApiError> {
        if files.is_empty() {
            return self.create_message(channel, message).await;
        }
        if files.len() > 10 || files.iter().map(|f| f.bytes.len()).sum::<usize>() > 25 * 1024 * 1024
        {
            return Err(ApiError::InvalidArgument(
                "At most 10 files and 25 MiB total are supported",
            ));
        }
        if !message.content.is_empty() {
            crate::client::validate_content(&message.content)?;
        }
        if message.nonce.is_empty() || message.nonce.len() > 25 {
            return Err(ApiError::InvalidArgument("nonce must contain 1-25 bytes"));
        }
        let mut payload = serde_json::to_value(message).map_err(ApiError::Decode)?;
        let names: Vec<String> = files
            .iter()
            .map(|f| {
                f.filename
                    .chars()
                    .map(|c| {
                        if matches!(c, '\r' | '\n' | '"' | '\\') {
                            '_'
                        } else {
                            c
                        }
                    })
                    .collect::<String>()
            })
            .collect();
        if names.iter().any(|name| name.is_empty() || name.len() > 255) {
            return Err(ApiError::InvalidArgument("Invalid attachment filename"));
        }
        payload["attachments"] = serde_json::Value::Array(
            names
                .iter()
                .enumerate()
                .map(|(index, name)| serde_json::json!({"id":index,"filename":name}))
                .collect(),
        );
        let boundary = format!(
            "dc-upload-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        if files.iter().any(|f| {
            f.bytes
                .windows(boundary.len())
                .any(|part| part == boundary.as_bytes())
        }) {
            return Err(ApiError::InvalidArgument(
                "Attachment contains the MIME boundary; retry",
            ));
        }
        let mut body=format!("--{boundary}\r\nContent-Disposition: form-data; name=\"payload_json\"\r\nContent-Type: application/json\r\n\r\n{payload}\r\n").into_bytes();
        for (index, file) in files.iter().enumerate() {
            body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"files[{index}]\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n",names[index]).as_bytes());
            body.extend_from_slice(&file.bytes);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        let result = self
            .post_body(
                Route::create_message(channel),
                body,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .await?;
        serde_json::from_slice(&result).map_err(ApiError::Decode)
    }
}
