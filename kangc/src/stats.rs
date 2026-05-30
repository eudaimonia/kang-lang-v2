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

#[derive(Serialize, Debug, Clone)]
pub struct SemanticStats {
    pub duration_us: u64,
    pub error_count: usize,
    pub warning_count: usize,
    pub symbol_count: usize,
    pub type_check_passes: usize,
    pub type_check_failures: usize,
}

#[derive(Serialize, Debug, Clone)]
pub struct CodeGenStats {
    pub duration_us: u64,
    pub llvm_ir_bytes: usize,
    pub llvm_instruction_count: usize,
    pub llvm_basic_block_count: usize,
    pub llvm_function_count: usize,
    pub runtime_check_insertions: usize,
}

/// 代码生成完整产物
#[derive(Serialize, Debug, Clone)]
pub struct CodeGenResult {
    pub ir_text: String,
    pub stats: CodeGenStats,
}
