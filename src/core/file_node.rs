use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// FileNode — 文件基准
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileNode {
    /// 文件路径（相对于仓库根）
    pub file_path: PathBuf,
    /// 文件内容的 Blake3 哈希
    pub base_hash: [u8; 32],
}

impl FileNode {
    pub fn new(file_path: PathBuf, content: &[u8]) -> Self {
        let hash = blake3::hash(content);
        FileNode {
            file_path,
            base_hash: *hash.as_bytes(),
        }
    }

    /// 返回文件路径的字符串表示
    pub fn path_str(&self) -> &str {
        self.file_path.to_str().unwrap_or("")
    }
}
