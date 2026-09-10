# -*- coding: utf-8 -*-
"""扫描项目 .py 源码（字符串字面量）与 about/*.md 界面文档，提取全部字符，分类汇总到文件。"""
import ast
import os

ROOT = os.path.dirname(os.path.abspath(__file__))
EXCLUDE_DIRS = {".venv", "venv", "packages", "legacy-v2026.08.20.1943",
                ".git", "__pycache__", ".idea", ".vscode", "build", "dist",
                "android", "node_modules", ".trae-cn", ".py312-src"}

all_chars = set()
file_count = 0

for dirpath, dirnames, filenames in os.walk(ROOT):
    dirnames[:] = [d for d in dirnames if d not in EXCLUDE_DIRS]
    for fn in filenames:
        fp = os.path.join(dirpath, fn)
        try:
            if fn.endswith(".py"):
                # Python 源码：用 AST 提取所有字符串字面量（忽略注释/docstring）
                with open(fp, "r", encoding="utf-8") as f:
                    src = f.read()
                tree = ast.parse(src, filename=fp)
                file_count += 1
                for node in ast.walk(tree):
                    if isinstance(node, ast.Constant) and isinstance(node.value, str):
                        all_chars.update(node.value)
            elif fn.endswith(".md") and os.path.basename(dirpath) == "about":
                # about 目录下的 markdown 文档：界面关于页直接渲染，全文扫描
                with open(fp, "r", encoding="utf-8") as f:
                    all_chars.update(f.read())
                file_count += 1
        except Exception:
            pass

# 过滤无效替换字符 U+FFFD（非 UTF-8 字节产生的噪声，无实际显示意义）
all_chars.discard("\ufffd")
# 过滤无显示意义的 C0 控制字符（U+0000-U+001F、U+007F，保留 \n\r\t 供"空白"类别）
all_chars = {c for c in all_chars if c in "\n\r\t" or ord(c) >= 0x20}


def classify(ch):
    o = ord(ch)
    if 0x4E00 <= o <= 0x9FFF:
        return "中文汉字"
    elif 0x3400 <= o <= 0x4DBF:
        return "中文扩展A"
    elif 0xFF00 <= o <= 0xFFEF:
        return "全角符号"
    elif 0x3000 <= o <= 0x303F:
        return "CJK标点"
    elif 0x0041 <= o <= 0x005A or 0x0061 <= o <= 0x007A:
        return "英文字母"
    elif 0x0030 <= o <= 0x0039:
        return "数字"
    elif 0x0020 <= o <= 0x007E:
        return "ASCII符号"
    elif o in (0x0A, 0x0D, 0x09):
        return "空白"
    elif 0x1F600 <= o <= 0x1FAFF:
        return "Emoji"
    else:
        return f"其他(U+{o:04X})"


categories = {}
for ch in sorted(all_chars):
    cat = classify(ch)
    categories.setdefault(cat, []).append(ch)

# 类别中文名 → 英文安全文件名（不含路径分隔符）
CAT_FILENAME = {
    "中文汉字": "cjk_han",
    "中文扩展A": "cjk_ext_a",
    "CJK标点": "cjk_punct",
    "全角符号": "fullwidth",
    "英文字母": "ascii_letters",
    "数字": "digits",
    "ASCII符号": "ascii_symbols",
    "空白": "whitespace",
    "Emoji": "emoji",
}
# 固定类别优先顺序
ordered = ["中文汉字", "中文扩展A", "CJK标点", "全角符号",
           "英文字母", "数字", "ASCII符号", "空白", "Emoji"]

# 1) 分类明细汇总文件
out_path = os.path.join(ROOT, "chars_used.txt")
with open(out_path, "w", encoding="utf-8") as f:
    f.write(f"扫描文件数: {file_count}\n")
    f.write(f"总唯一字符数: {len(all_chars)}\n")
    f.write("=" * 60 + "\n\n")
    for cat in ordered:
        if cat in categories:
            chars = categories[cat]
            f.write(f"【{cat}】({len(chars)}个)\n")
            f.write("".join(chars) + "\n\n")
    for cat in sorted(categories):
        if cat.startswith("其他"):
            chars = categories[cat]
            f.write(f"【{cat}】({len(chars)}个)\n")
            f.write("".join(chars) + "\n\n")

# 2) 按类别分别输出独立 txt 文件（统一放到 chars_categories/ 子目录）
cat_dir = os.path.join(ROOT, "chars_categories")
os.makedirs(cat_dir, exist_ok=True)
written_files = []
for cat in ordered:
    if cat not in categories:
        continue
    chars = categories[cat]
    fname = CAT_FILENAME.get(cat, "other")
    fp_cat = os.path.join(cat_dir, f"chars_{fname}.txt")
    with open(fp_cat, "w", encoding="utf-8", newline="") as f:
        f.write("".join(chars))
    written_files.append((cat, len(chars), fp_cat))

# 所有"其他"子类别字符合并、排序后写入单个文件（避免每个 U+XXXX 建一个文件）
other_chars = []
for cat in sorted(c for c in categories if c.startswith("其他")):
    other_chars.extend(categories[cat])
if other_chars:
    other_chars = sorted(other_chars)
    fp_other = os.path.join(cat_dir, "chars_other.txt")
    with open(fp_other, "w", encoding="utf-8", newline="") as f:
        f.write("".join(other_chars))
    written_files.append(("其他(合并)", len(other_chars), fp_other))

print(f"已写入汇总: {out_path}")
print(f"已写入分类文件: {cat_dir}（共 {len(written_files)} 个）")
for cat, cnt, fp_cat in written_files:
    print(f"  [{cat}] {cnt}个 → {os.path.basename(fp_cat)}")
print(f"扫描文件数: {file_count}, 总字符数: {len(all_chars)}")

# 过滤控制字符（换行/回车/制表），排序后拼成单个字符串输出
visible = sorted(c for c in all_chars if c not in "\n\r\t")
joined = "".join(visible)
print(f"\n可见字符数: {len(visible)}")
print("=" * 60)
print(joined)

# 拼接字符串独立写入纯文本文件（无表头/分类，便于直接用于字体子集化等工具）
set_path = os.path.join(ROOT, "chars_set.txt")
with open(set_path, "w", encoding="utf-8", newline="") as f:
    f.write(joined)
print(f"\n拼接字符串已写入: {set_path}")
