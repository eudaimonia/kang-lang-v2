#!/bin/bash
# run_algorithms.sh — 经典算法示例执行器
#
# 用法:
#   ./examples/algorithms/run_algorithms.sh              # 执行全部示例
#   ./examples/algorithms/run_algorithms.sh --verbose    # 详细输出
#   KANGC="kangc" ./examples/algorithms/run_algorithms.sh  # 指定编译器路径

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

echo -e "${BOLD}=== Kang Classic Algorithms ===${RESET}"
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

# ── 算法列表 ────────────────────────────────────────────────────────────────────

algorithms=(
    "fibonacci:斐波那契数列 (迭代)"
    "factorial:阶乘 (递归+迭代)"
    "gcd_lcm:最大公约数/最小公倍数"
    "prime_sieve:埃拉托斯特尼筛法"
    "binary_search:二分查找"
    "bubble_sort:冒泡排序"
    "palindrome:回文检测"
    "fast_pow:快速幂 O(log n)"
    "hanoi:汉诺塔问题"
    "prime_factor:质因数分解"
)

for entry in "${algorithms[@]}"; do
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
