#!/usr/bin/env bash
# test.sh — verify-release-tag composite action 的本地断言套件。
#
# 背景：actionlint 不覆盖 composite action 的 run 块（阴性对照实验证实，见
# add-tag-release-pipeline tasks 4.1），而「tag↔Cargo.toml 版本一致性 + is_prerelease
# 判定」是发布流水线第一道门，逻辑错了要么放过坏 tag 要么拦住好 tag。本脚本把
# action.yml 里的 run 块原样提取出来（不是复制品——随 action.yml 演进自动跟进），
# 用 Cargo.toml 变体夹具 + 假 tag 逐例断言，不需要网络、不需要 runner。
#
# 用法：bash .github/actions/verify-release-tag/test.sh
# 依赖：bash、python3 + PyYAML（与 add-tag-release-pipeline tasks 4.1 的 YAML
#       解析门禁同一依赖）。

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
action="$here/action.yml"
root="$(cd "$here/../../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fixtures="$work/fixtures"
mkdir -p "$fixtures"

# 1) 原样提取 run 块（唯一带 run 的 step）
python3 - "$action" "$work/run.sh" <<'PY'
import sys, yaml
action, out = sys.argv[1], sys.argv[2]
with open(action, encoding="utf-8") as f:
    doc = yaml.safe_load(f)
runs = doc["runs"]
assert runs["using"] == "composite", "not a composite action?"
steps = [s for s in runs["steps"] if "run" in s]
assert len(steps) == 1, f"expected exactly 1 run step, got {len(steps)}"
with open(out, "w", encoding="utf-8") as f:
    f.write(steps[0]["run"])
PY

# 2) 夹具：mk <名字> <带\n的 Cargo.toml 内容>；仓库根真件直接拷贝
mk() { printf '%b' "$2" > "$fixtures/$1"; }
mk no-version       '[package]\nname = "x"\n'
mk mismatch         '[package]\nname = "x"\nversion = "0.1.0"\n'
mk rc               '[package]\nname = "x"\nversion = "0.2.0-rc.1"\n'
mk single-quote     "[package]\nname = \"x\"\nversion = '0.1.0'\n"
mk no-space         '[package]\nname = "x"\nversion="0.1.0"\n'
mk trailing-cmt     '[package]\nname = "x"\nversion = "0.1.0" # the release version\n'
mk comment-decoy    '# version = "9.9.9"  (decoy comment)\n[package]\nname = "x"\nversion = "0.1.0"\n'
mk workspace-inherit '[package]\nname = "x"\nversion.workspace = true\n\n[workspace.package]\nversion = "0.1.0"\n'
cp "$root/Cargo.toml" "$fixtures/repo-real"

# 3) 逐例执行：expect_rc: 0 或 1(非零)；expect_pre: is_prerelease 期望值，空=不关心
pass=0; fail=0
run_case() {
  local name="$1" fixture="$2" tag="$3" expect_rc="$4" expect_pre="$5" note="${6:-}"
  local dir="$work/$name"; mkdir -p "$dir"
  cp "$fixtures/$fixture" "$dir/Cargo.toml"
  local outf="$dir/github_output" rc=0 pre=""
  ( cd "$dir" && GITHUB_REF_NAME="$tag" GITHUB_OUTPUT="$outf" bash "$work/run.sh" ) \
    >"$dir/out" 2>"$dir/err" || rc=$?
  [[ -f "$outf" ]] && pre="$(sed -n 's/^is_prerelease=//p' "$outf" | head -1)"
  local ok=1
  if [[ "$expect_rc" == "0" ]]; then [[ $rc -eq 0 ]] || ok=0; else [[ $rc -ne 0 ]] || ok=0; fi
  [[ -z "$expect_pre" || "$pre" == "$expect_pre" ]] || ok=0
  if [[ $ok -eq 1 ]]; then
    pass=$((pass+1)); printf 'PASS  %-18s fixture=%-18s tag=%-13s rc=%s is_prerelease=%s\n' \
      "$name" "$fixture" "$tag" "$rc" "${pre:-<unset>}"
  else
    fail=$((fail+1)); printf 'FAIL  %-18s fixture=%-18s tag=%-13s rc=%s is_prerelease=%s (expect rc=%s pre=%s)\n' \
      "$name" "$fixture" "$tag" "$rc" "${pre:-<unset>}" "$expect_rc" "${expect_pre:-<any>}"
    sed 's/^/      | /' "$dir/err" | head -5
  fi
  [[ -z "$note" ]] || printf '      note: %s\n' "$note"
  return 0
}

run_case repo-real       repo-real         v0.1.0       0 "false" "仓库根真件：version 在 [package] 首 ^version 行"
run_case mismatch        mismatch          v9.9.9       1 ""      "spec：不一致 tag 快速失败"
run_case rc-prerelease   rc                v0.2.0-rc.1  0 "true"  "spec：rc tag → is_prerelease=true"
run_case no-v-prefix     mismatch          0.1.0        1 ""      "防御：缺 v 前缀"
run_case non-version-tag mismatch          release-1    1 ""      "防御：非 v<数字> 形态"
run_case missing-version no-version        v0.1.0       1 ""      "防御：Cargo.toml 无 ^version 行（set -e 提前终止，友好报错分支不可达，见报告）"
run_case single-quote    single-quote      v0.1.0       1 ""      "已知边界：TOML 单引号字面串不被捕获 → 整行 passthrough → 比对失败（fail-safe，拒真不纳假）"
run_case no-space        no-space          v0.1.0       0 "false" "version=\"0.1.0\" 无空格形态"
run_case trailing-cmt    trailing-cmt      v0.1.0       0 "false" "行尾注释"
run_case comment-decoy   comment-decoy     v0.1.0       0 "false" "# 注释行不参与 ^version 匹配"
run_case workspace-inherit workspace-inherit v0.1.0     0 "false" "version.workspace 继承：落点为 [workspace.package] 列 0 version（注意 root 需非 virtual manifest 才有 [package]）"

echo
echo "result: $pass passed, $fail failed"
[[ $fail -eq 0 ]]
