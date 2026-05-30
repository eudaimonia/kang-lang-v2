// 统计数据定义 + serde 序列化，全模块共用

use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize, Debug, Clone)]
pub struct SourceStats {
    pub file_path: String,
    pub total_bytes: usize,
    pub total_lines: usize,
}

#[derive(Serialize, Debug, Clone)]
pub struct LexStats {
    pub duration_us: u64,
    pub token_count: usize,
    pub token_counts_by_kind: HashMap<String, usize>,
    pub comment_bytes: usize,
}

#[derive(Serialize, Debug, Clone)]
pub struct ParseStats {
    pub duration_us: u64,
    pub ast_node_count: usize,
    pub ast_max_depth: usize,
    pub node_counts_by_kind: HashMap<String, usize>,
    pub func_count: usize,
    pub struct_count: usize,
}
