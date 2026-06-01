#!/usr/bin/env bash
# 项目代码数量统计
# 统计各组件 .rs 文件的行数，区分代码、注释、空行
set -euo pipefail

cd "$(dirname "$0")/.."

echo "===== Kang v2 代码统计 ====="
echo

# 按组件统计
for component in kangc kangrt parser_prototype; do
    files=$(find "$component" -name '*.rs' 2>/dev/null | sort)
    if [ -z "$files" ]; then
        continue
    fi

    case "$component" in
        kangc) label="kangc (编译器)" ;;
        kangrt) label="kangrt (运行时)" ;;
        parser_prototype) label="parser_prototype (词法原型)" ;;
    esac

    echo "--- $label ---"

    total_lines=0
    code_lines=0
    comment_lines=0
    blank_lines=0
    file_count=0

    for f in $files; do
        file_count=$((file_count + 1))
        n=$(wc -l < "$f")
        total_lines=$((total_lines + n))

        # 统计空行（只包含空白字符的行）
        b=$(grep -c '^[[:space:]]*$' "$f" || true)
        blank_lines=$((blank_lines + b))

        # 统计注释行（// 或 /// 或 //! 开头，忽略行内注释）
        c=$(grep -c '^\s*//' "$f" || true)
        comment_lines=$((comment_lines + c))

        # 统计行内 /* */ 注释和 doc 注释
        # / * 注释跨行情况用简单近似：以 * / 结尾的行
        cb=$(grep -c '^\s*\*' "$f" || true)
        # 块注释开始行 /* （非行尾结束）
        ob=$(grep -c '/\*' "$f" || true)
        comment_lines=$((comment_lines + cb + ob))
    done

    code_lines=$((total_lines - blank_lines - comment_lines))

    printf "  文件数:     %3d\n" "$file_count"
    printf "  总行数:     %5d\n" "$total_lines"
    printf "  代码行:     %5d  (%d%%)\n" "$code_lines" "$((code_lines * 100 / total_lines))"
    printf "  注释行:     %5d  (%d%%)\n" "$comment_lines" "$((comment_lines * 100 / total_lines))"
    printf "  空行:       %5d  (%d%%)\n" "$blank_lines" "$((blank_lines * 100 / total_lines))"
    echo
done

# 全部汇总
echo "--- 汇总（含 .kang 示例） ---"
kang_files=$(find . \( -name target -o -name .git \) -prune -o -name '*.kang' -print | wc -l | tr -d ' ')
kang_lines=$(find . \( -name target -o -name .git \) -prune -o -name '*.kang' -print -exec cat {} + | wc -l | tr -d ' ')
echo "  .kang 示例文件数: $kang_files"
echo "  .kang 示例行数: $kang_lines"

shell_files=$(find . \( -name target -o -name .git -o -name scripts \) -prune -o -name '*.sh' -print | wc -l | tr -d ' ')
shell_lines=$(find . \( -name target -o -name .git -o -name scripts \) -prune -o -name '*.sh' -exec cat {} + | wc -l | tr -d ' ')
echo "  .sh 脚本文件数: $shell_files"
echo "  .sh 脚本行数: $shell_lines"
