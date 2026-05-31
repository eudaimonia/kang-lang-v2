#!/bin/bash
# run_data_structures.sh — 经典数据结构示例执行器
#
# 用法:
#   ./examples/data_structures/run_data_structures.sh              # 执行全部示例
#   ./examples/data_structures/run_data_structures.sh --verbose    # 详细输出
#   KANGC="kangc" ./examples/data_structures/run_data_structures.sh  # 指定编译器路径

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

echo -e "${BOLD}=== Kang Data Structures ===${RESET}"
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

# ── 数据结构列表 ────────────────────────────────────────────────────────────────

data_structures=(
    "stack:数组栈 (LIFO)"
    "queue:循环队列 (FIFO)"
    "deque:双端队列"
    "ring_buffer:环形缓冲区"
    "linked_list:单向链表 (索引指针)"
    "bounded_stack:有界栈 + O(1) 最小值"
    "two_stack_queue:双栈模拟队列"
    "counter:频率计数器"
    "ordered_set:有序集合 (二分查找)"
    "binary_heap:最小堆 + 堆排序"
)

for entry in "${data_structures[@]}"; do
    IFS=':' read -r file desc <<< "$entry"
    src="$SCRIPT_DIR/${file}.kang"
    binary="$TMPDIR/$file"
    log="$TMPDIR/${file}.log"

    printf "  %-20s %s\n" "$file" "$desc"
    printf "    "

    compile_cmd="$KANGC build "$src" -o "$binary""
    if $compile_cmd >"$log" 2>&1; then
        if "$binary" >>"$log" 2>&1; then
            echo -e "${GREEN}PASS${RESET}"
            ((PASS++))
        else
            rc=$?
            echo -e "${RED}FAIL${RESET} (exit code $rc)"
            ((FAIL++))
            FAILED_TESTS+=("$file")
        fi
    else
        rc=$?
        echo -e "${RED}FAIL${RESET} (compile error, exit $rc)"
        ((FAIL++))
        FAILED_TESTS+=("$file")
        echo "    --- 编译错误 ---"
        cat "$log" | sed 's/^/      /'
        echo "    -----------------"
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
