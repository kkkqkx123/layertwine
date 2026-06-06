use crate::StorageResult;
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct SqliteStorage {
    pub conn: Arc<Mutex<Connection>>,
}

impl Clone for SqliteStorage {
    fn clone(&self) -> Self {
        SqliteStorage {
            conn: self.conn.clone(),
        }
    }
}

impl crate::storage::repository::AtomicOps for SqliteStorage {
    fn with_atomic<F, T>(&self, f: F) -> StorageResult<T>
    where
        F: FnOnce(&Self) -> StorageResult<T>,
    {
        self.with_conn(|conn| {
            conn.execute_batch("SAVEPOINT atomic_savepoint;")?;
            match f(self) {
                Ok(value) => {
                    conn.execute_batch("RELEASE SAVEPOINT atomic_savepoint;")?;
                    Ok(value)
                }
                Err(e) => {
                    conn.execute_batch("ROLLBACK TO SAVEPOINT atomic_savepoint;")?;
                    Err(e)
                }
            }
        })
    }
}

impl SqliteStorage {
    pub fn new_in_memory() -> StorageResult<Self> {
        let conn = Connection::open_in_memory()?;
        crate::storage::migrations::initialize_database(&conn)?;
        Ok(SqliteStorage {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn new(path: &Path) -> StorageResult<Self> {
        let conn = Connection::open(path)?;
        crate::storage::migrations::initialize_database(&conn)?;
        Ok(SqliteStorage {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn new_full(path: &Path) -> StorageResult<Self> {
        let conn = Connection::open(path)?;
        crate::storage::migrations::initialize_full(&conn)?;
        Ok(SqliteStorage {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn new_with_connection_arc(conn: &Arc<Mutex<Connection>>) -> Self {
        SqliteStorage { conn: conn.clone() }
    }

    pub fn share(&self) -> Self {
        SqliteStorage {
            conn: self.conn.clone(),
        }
    }

    pub fn with_conn<F, T>(&self, f: F) -> StorageResult<T>
    where
        F: FnOnce(&Connection) -> StorageResult<T>,
    {
        let conn = self.conn.lock().unwrap();
        f(&conn)
    }

    pub fn with_transaction<F, T>(&self, f: F) -> StorageResult<T>
    where
        F: FnOnce(&Connection) -> StorageResult<T>,
    {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN TRANSACTION;")?;
        match f(&conn) {
            Ok(result) => {
                conn.execute_batch("COMMIT;")?;
                Ok(result)
            }
            Err(e) => {
                conn.execute_batch("ROLLBACK;")?;
                Err(e)
            }
        }
    }
}
