#!/bin/bash
# run_multi.sh — 多文件模块编译链接示例执行器
#
# 用法:
#   ./examples/multi/run_multi.sh              # 执行全部示例
#   ./examples/multi/run_multi.sh --verbose    # 详细输出
#   KANGC="kangc" ./examples/multi/run_multi.sh  # 指定编译器路径
#
# 每个示例的 main() 返回 0 表示通过, 非 0 表示失败。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

KANGC="${KANGC:-cargo run --release -p kangc --}"
VERBOSE=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --verbose|-v) VERBOSE=true; shift ;;
        *) echo "未知参数: $1"; exit 1 ;;
    esac
done

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

PASS=0
FAIL=0
FAILED_TESTS=()

BOLD="\033[1m"
GREEN="\033[32m"
RED="\033[31m"
RESET="\033[0m"

echo -e "${BOLD}=== Kang Multi-File Examples ===${RESET}"
echo "日期: $(date '+%Y-%m-%d %H:%M:%S')"
echo ""

# 确保编译器已构建
echo -n "构建 kangc ... "
if ! (cd "$PROJECT_ROOT" && cargo build --release -p kangc --quiet 2>&1); then
    echo -e "${RED}编译器构建失败${RESET}"
    exit 1
fi
echo "OK"
echo ""

# ── 示例列表 ────────────────────────────────────────────────────────────────────

examples=(
    "01_simple_import:单文件导入:math.kang → main.kang"
    "02_multi_module:多模块导入:strings.kang + calc.kang → main.kang"
    "03_struct_export:结构体导出:geom.kang (Point struct) → main.kang"
    "04_chained_import:链式导入:base.kang → mid.kang → main.kang"
    "05_lib_style:库风格依赖:calc.kang + fmt.kang → main.kang (菱形依赖)"
)

for entry in "${examples[@]}"; do
    IFS=':' read -r dir name desc <<< "$entry"
    log="$TMPDIR/${dir}.log"
    binary="$TMPDIR/${dir}"

    printf "  %-25s %s\n" "$name" "$desc"
    printf "    "

    # 编译 (入口文件始终是 main.kang, imports 自动解析)
    compile_cmd="$KANGC build "$SCRIPT_DIR/$dir/main.kang" -o "$binary""
    if $compile_cmd >"$log" 2>&1; then
        # 执行
        if "$binary" >>"$log" 2>&1; then
            echo -e "${GREEN}PASS${RESET}"
            ((PASS++))
        else
            rc=$?
            echo -e "${RED}FAIL${RESET} (exit code $rc)"
            ((FAIL++))
            FAILED_TESTS+=("$name")
        fi
    else
        rc=$?
        echo -e "${RED}FAIL${RESET} (compile error, exit $rc)"
        ((FAIL++))
        FAILED_TESTS+=("$name")
    fi

    if $VERBOSE; then
        echo "    --- 输出 ---"
        cat "$log" | sed 's/^/      /'
        echo "    ------------"
    fi
done

echo ""
echo -e "${BOLD}=== 结果: ${GREEN}$PASS 通过${RESET}, ${RED}$FAIL 失败${RESET} ===${BOLD}"

if [[ ${#FAILED_TESTS[@]} -gt 0 ]]; then
    echo ""
    echo -e "${RED}失败示例:${RESET}"
    for t in "${FAILED_TESTS[@]}"; do
        echo "  - $t"
    done
    exit 1
fi
