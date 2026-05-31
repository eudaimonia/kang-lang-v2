#!/bin/bash
# run_e2e.sh — Kang 端到端测试套件执行器
#
# 用法:
#   ./tests/e2e/run_e2e.sh              # 执行全部测试
#   ./tests/e2e/run_e2e.sh --verbose    # 详细输出 (显示每个测试的 stdout)
#   ./tests/e2e/run_e2e.sh --target=x86_64-unknown-linux-musl  # 交叉编译
#   KANGC="kangc" ./tests/e2e/run_e2e.sh  # 指定编译器路径
#
# 每个 .kang 文件的 main() 返回 0 表示通过, 非 0 表示失败。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

KANGC="${KANGC:-cargo run --release -p kangc --}"
VERBOSE=false
TARGET_FLAG=""

# 解析参数
while [[ $# -gt 0 ]]; do
    case "$1" in
        --verbose|-v) VERBOSE=true; shift ;;
        --target=*) TARGET_FLAG="$1"; shift ;;
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
YELLOW="\033[33m"
RESET="\033[0m"

echo -e "${BOLD}=== Kang E2E Test Suite ===${RESET}"
echo "日期: $(date '+%Y-%m-%d %H:%M:%S')"
echo "目录: $SCRIPT_DIR"
echo ""

# 收集所有测试文件, 按名称排序
shopt -s nullglob
test_files=("$SCRIPT_DIR"/*.kang)
shopt -u nullglob

if [[ ${#test_files[@]} -eq 0 ]]; then
    echo "未找到测试文件 (*.kang)"
    exit 1
fi

# 排序
IFS=$'\n' test_files=($(sort <<<"${test_files[*]}"))
unset IFS

echo "发现 ${#test_files[@]} 个测试文件"
echo ""

# 先确保编译器已构建
echo -n "构建 kangc ... "
if ! (cd "$PROJECT_ROOT" && cargo build --release -p kangc --quiet 2>&1); then
    echo -e "${RED}编译器构建失败${RESET}"
    exit 1
fi
echo "OK"
echo ""

# ── 执行每个测试 ────────────────────────────────────────────────────────────────

for test_file in "${test_files[@]}"; do
    name="$(basename "$test_file" .kang)"
    binary="$TMPDIR/$name"
    log="$TMPDIR/${name}.log"

    printf "  %-35s " "$name"

    # 编译
    compile_cmd="$KANGC build "$test_file" -o "$binary" $TARGET_FLAG"
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
            if $VERBOSE; then
                echo "    --- stdout/stderr ---"
                cat "$log" | sed 's/^/    /'
                echo "    ---------------------"
            fi
        fi
    else
        rc=$?
        echo -e "${RED}FAIL${RESET} (compile error, exit $rc)"
        ((FAIL++))
        FAILED_TESTS+=("$name")
        echo "    --- 编译错误 ---"
        cat "$log" | sed 's/^/    /'
        echo "    -----------------"
    fi
done

echo ""
echo -e "${BOLD}=== 结果: ${GREEN}$PASS 通过${RESET}, ${RED}$FAIL 失败${RESET} ===${BOLD}"

# 列出失败测试
if [[ ${#FAILED_TESTS[@]} -gt 0 ]]; then
    echo ""
    echo -e "${RED}失败测试:${RESET}"
    for t in "${FAILED_TESTS[@]}"; do
        echo "  - $t"
    done
    echo ""
    echo "重新运行失败测试:"
    echo "  KANGC=\"$KANGC\" $0 --verbose"
fi
