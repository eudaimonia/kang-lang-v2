// 编译器统计数据定义 + serde 序列化，全模块共用
// 每个编译阶段有独立的统计结构体，最终聚合为 CompilerStats 供 JSON 输出

use serde::Serialize;
use std::collections::HashMap;

/// 源文件统计信息
#[derive(Serialize, Debug, Clone, Default)]
pub struct SourceStats {
    pub file_path: String,       // 源文件路径
    pub total_bytes: usize,      // 文件字节数
    pub total_lines: usize,      // 文件行数
}

/// 词法分析阶段统计
#[derive(Serialize, Debug, Clone, Default)]
pub struct LexStats {
    pub duration_us: u64,                        // 耗时（微秒）
    pub token_count: usize,                      // token 总数
    pub token_counts_by_kind: HashMap<String, usize>,  // 各 token 类型计数
    pub comment_bytes: usize,                    // 注释字节数
}

/// 语法分析阶段统计
#[derive(Serialize, Debug, Clone, Default)]
pub struct ParseStats {
    pub duration_us: u64,                        // 耗时（微秒）
    pub ast_node_count: usize,                   // AST 节点总数
    pub ast_max_depth: usize,                    // AST 最大嵌套深度
    pub node_counts_by_kind: HashMap<String, usize>,   // 各节点类型计数
    pub func_count: usize,                       // 函数定义数量
    pub struct_count: usize,                     // 结构体定义数量
}

/// 语义分析阶段统计
#[derive(Serialize, Debug, Clone, Default)]
pub struct SemanticStats {
    pub duration_us: u64,        // 耗时（微秒）
    pub error_count: usize,      // 错误总数
    pub warning_count: usize,    // 警告总数
    pub symbol_count: usize,     // 符号表条目数
    pub type_check_passes: usize,    // 类型检查通过次数
    pub type_check_failures: usize,  // 类型检查失败次数
}

/// 代码生成阶段统计
#[derive(Serialize, Debug, Clone, Default)]
pub struct CodeGenStats {
    pub duration_us: u64,               // 耗时（微秒）
    pub llvm_ir_bytes: usize,           // 生成的 LLVM IR 字节数
    pub llvm_instruction_count: usize,  // LLVM IR 指令数
    pub llvm_basic_block_count: usize,  // LLVM 基本块数
    pub llvm_function_count: usize,     // LLVM 函数数
    pub runtime_check_insertions: usize,// 插入的运行时安全检查数
}

/// 全编译管线统计聚合
#[derive(Serialize, Debug, Clone)]
pub struct CompilerStats {
    pub source: SourceStats,
    pub lex: LexStats,
    pub parse: ParseStats,
    pub semantic: SemanticStats,
    pub codegen: CodeGenStats,
}

impl Default for CompilerStats {
    fn default() -> Self {
        CompilerStats {
            source: SourceStats::default(),
            lex: LexStats::default(),
            parse: ParseStats::default(),
            semantic: SemanticStats::default(),
            codegen: CodeGenStats::default(),
        }
    }
}

/// 代码生成完整产物
#[derive(Serialize, Debug, Clone)]
pub struct CodeGenResult {
    pub ir_text: String,
    pub stats: CodeGenStats,
    pub object_file: Option<String>,
}
