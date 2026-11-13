//! Optional per-account SQLite history cache. No READY envelopes or credentials.
use dc_protocol::{Snowflake, model::Message};
use rusqlite::{Connection, params};

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("database operation failed: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("cached message could not be decoded")]
    Decode(#[from] serde_json::Error),
}
pub trait HistoryCache {
    fn history(&self, channel: Snowflake, limit: usize) -> Result<Vec<Message>, CacheError>;
    fn remember(&mut self, message: &Message) -> Result<(), CacheError>;
    fn remove(&mut self, channel: Snowflake, message: Snowflake) -> Result<(), CacheError>;
}
pub struct SqliteCache {
    connection: Connection,
}
impl SqliteCache {
    pub fn open(path: &std::path::Path) -> Result<Self, CacheError> {
        Self::from_connection(Connection::open(path)?)
    }
    pub fn memory() -> Result<Self, CacheError> {
        Self::from_connection(Connection::open_in_memory()?)
    }
    fn from_connection(connection: Connection) -> Result<Self, CacheError> {
        connection.execute_batch("PRAGMA secure_delete=ON; CREATE TABLE IF NOT EXISTS messages (channel TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY(channel,id));")?;
        Ok(Self { connection })
    }
    pub fn clear(&mut self) -> Result<(), CacheError> {
        self.connection.execute("DELETE FROM messages", [])?;
        self.connection.execute_batch("VACUUM")?;
        Ok(())
    }
}
impl HistoryCache for SqliteCache {
    fn history(&self, channel: Snowflake, limit: usize) -> Result<Vec<Message>, CacheError> {
        let mut statement = self
            .connection
            .prepare("SELECT body FROM messages WHERE channel=?1 ORDER BY id DESC LIMIT ?2")?;
        let rows = statement
            .query_map(params![channel.to_string(), limit.min(500) as i64], |row| {
                row.get::<_, String>(0)
            })?;
        let mut messages = Vec::new();
        for row in rows {
            messages.push(serde_json::from_str(&row?)?);
        }
        messages.reverse();
        Ok(messages)
    }
    fn remember(&mut self, message: &Message) -> Result<(), CacheError> {
        let mut safe = message.clone();
        safe.extra.clear();
        let body = serde_json::to_string(&safe)?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO messages(channel,id,body) VALUES (?1,?2,?3)",
            params![
                message.channel_id.to_string(),
                format!("{:020}", message.id.get()),
                body
            ],
        )?;
        tx.execute("DELETE FROM messages WHERE channel=?1 AND id NOT IN (SELECT id FROM messages WHERE channel=?1 ORDER BY id DESC LIMIT 500)",[message.channel_id.to_string()])?;
        tx.commit()?;
        Ok(())
    }
    fn remove(&mut self, channel: Snowflake, message: Snowflake) -> Result<(), CacheError> {
        self.connection.execute(
            "DELETE FROM messages WHERE channel=?1 AND id=?2",
            params![channel.to_string(), format!("{:020}", message.get())],
        )?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_is_bounded_orders_ids_and_applies_deletes() {
        let mut cache = SqliteCache::memory().unwrap();
        let mut message: Message=serde_json::from_value(serde_json::json!({"id":"9","channel_id":"1","author":{"id":"2"},"content":"hello","sensitive_future_field":"not persisted"})).unwrap();
        cache.remember(&message).unwrap();
        message.id = Snowflake(10);
        cache.remember(&message).unwrap();
        let history = cache.history(Snowflake(1), 50).unwrap();
        assert_eq!(
            history.iter().map(|m| m.id.get()).collect::<Vec<_>>(),
            [9, 10]
        );
        assert!(history[0].extra.is_empty());
        cache.remove(Snowflake(1), Snowflake(9)).unwrap();
        assert_eq!(cache.history(Snowflake(1), 50).unwrap().len(), 1);
        cache.clear().unwrap();
        assert!(cache.history(Snowflake(1), 50).unwrap().is_empty());
    }
}
