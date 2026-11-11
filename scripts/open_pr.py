#!/usr/bin/env python3
"""把自己插件的条目提成 openidea-plugins 的 PR。

在插件项目根目录运行：

    python scripts/open_pr.py --dry-run    # 只打印计划
    python scripts/open_pr.py              # fork、推分支、开 PR

前提：
    - `publish/publishers/<publisher>.toml` 与 `publish/plugins/<id>.toml` 已写好；
    - 仓库已推到 GitHub，且 v<version> Release 已发布、附件 sha256 已核对；
    - 本机存有 GitHub 凭据（`git push` 能直接过，不用输密码）。

流程：确保 fork 存在 → 克隆 → 基于 upstream/main 建分支 → 放进两个条目 →
提交推送 → 调 API 开 PR。标题与正文取自 `publish/PR.md`（若存在）。
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
import tomllib
import urllib.error
import urllib.request
from pathlib import Path

for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8", errors="replace")
    except (AttributeError, ValueError):
        pass

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_UPSTREAM = "l0ng-ai/openidea-plugins"


def die(msg: str) -> None:
    print(f"  [x] {msg}")
    sys.exit(1)


def run(cmd: list[str], cwd: Path | None = None) -> subprocess.CompletedProcess:
    proc = subprocess.run(
        cmd,
        cwd=str(cwd) if cwd else None,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        env={**os.environ, "GIT_TERMINAL_PROMPT": "0"},
    )
    if proc.returncode != 0:
        print(f"  命令失败：{' '.join(cmd)}")
        print((proc.stdout + proc.stderr).strip()[:1200])
        sys.exit(1)
    return proc


def token() -> str:
    proc = subprocess.run(
        ["git", "credential", "fill"],
        input="protocol=https\nhost=github.com\n\n",
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        die("取不到 GitHub 凭据，先成功跑一次 git push（用 token 登录）")
    for line in proc.stdout.splitlines():
        if line.startswith("password="):
            return line.split("=", 1)[1]
    die("凭据里没有 token")


def api(tok: str, path: str, method: str = "GET", body: dict | None = None):
    req = urllib.request.Request(
        "https://api.github.com" + path,
        method=method,
        headers={
            "Authorization": f"Bearer {tok}",
            "Accept": "application/vnd.github+json",
            "User-Agent": "openidea-plugin-publish",
        },
    )
    if body is not None:
        req.add_header("Content-Type", "application/json")
        req.data = json.dumps(body).encode()
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            raw = resp.read()
            return resp.status, json.loads(raw) if raw else {}
    except urllib.error.HTTPError as exc:
        raw = exc.read()
        try:
            return exc.code, json.loads(raw) if raw else {}
        except json.JSONDecodeError:
            return exc.code, {"raw": raw.decode("utf-8", "replace")}


def block(text: str, heading: str, lang: str = "") -> str:
    """取出 `## heading` 之后第一个代码块的内容。"""
    idx = text.find(heading)
    if idx < 0:
        return ""
    m = re.search(r"```" + re.escape(lang) + r"\n(.*?)```", text[idx:], re.S)
    return m.group(1).strip() if m else ""


def repo_slug() -> tuple[str, str]:
    """从 origin 解析出 owner/repo。"""
    try:
        url = run(["git", "remote", "get-url", "origin"]).stdout.strip()
    except SystemExit:
        die("没有 origin 远程仓库，先推到 GitHub")
    m = re.search(r"github\.com[:/]([^/]+)/(.+?)(?:\.git)?$", url)
    if not m:
        die(f"看不懂 origin 地址：{url}")
    return m.group(1), m.group(2)


def main() -> None:
    ap = argparse.ArgumentParser(description="开 PR 到 openidea-plugins")
    ap.add_argument("--dry-run", action="store_true", help="只打印计划")
    ap.add_argument("--upstream", default=DEFAULT_UPSTREAM, help="目标仓库")
    ap.add_argument("--owner", help="你的 GitHub 登录名（默认从 origin 推）")
    ap.add_argument("--publisher", help="发布者前缀（默认取 id 的第一段）")
    ap.add_argument("--branch", help="分支名（默认 add-<id>）")
    args = ap.parse_args()

    manifest = tomllib.loads((ROOT / "plugin.toml").read_text(encoding="utf-8"))
    plugin_id = manifest["id"]
    version = manifest["version"]
    publisher = args.publisher or plugin_id.split(".")[0]

    owner, repo_name = repo_slug()
    if args.owner:
        owner = args.owner
    branch = args.branch or f"add-{plugin_id}"

    files = {
        f"publishers/{publisher}.toml": ROOT / "publish" / "publishers" / f"{publisher}.toml",
        f"plugins/{plugin_id}.toml": ROOT / "publish" / "plugins" / f"{plugin_id}.toml",
    }
    for rel, src in files.items():
        if not src.is_file():
            die(f"缺少 {src}")
        print(f"  本地 {rel}  ({src.stat().st_size} 字节)")

    pr_doc = ROOT / "publish" / "PR.md"
    if pr_doc.is_file():
        doc = pr_doc.read_text(encoding="utf-8")
        title = block(doc, "## 标题") or f"Add {plugin_id}"
        body = block(doc, "## 正文", "markdown")
        leftovers = [
            line.strip()
            for line in f"{title}\n{body}".splitlines()
            if "TODO" in line
        ]
        if leftovers:
            print("  [x] publish/PR.md 的标题或正文里还有占位：")
            for line in leftovers[:6]:
                print(f"      {line}")
            die("先把这些补完（审核会看：能力为什么要、测了什么）")
        if not body:
            die("publish/PR.md 的「## 正文」下没有 ```markdown 代码块")
        print(f"  正文 取自 publish/PR.md（{len(body)} 字符）")
    else:
        title = f"Add {plugin_id}"
        caps = manifest.get("capabilities", {})
        all_caps = list(caps.get("required", [])) + list(caps.get("optional", []))
        cap_lines = "\n".join(f"- `{c}`" for c in all_caps) or "- none"
        body = (
            f"### What it is\n\n"
            f"{manifest.get('name', plugin_id)} — {manifest.get('description', '')}\n\n"
            f"Source: https://github.com/{owner}/{repo_name} "
            f"({manifest.get('license', 'see repository')}).\n\n"
            f"### Capabilities\n\n{cap_lines}\n\n"
            f"### Notes\n\n"
            f"- Built from the linked source with `wasm32-wasip2` + component model "
            f"(`engines.openidea = \"{manifest.get('engines', {}).get('openidea', '')}\"`).\n"
            f"- Version {version}.\n"
        )
        print("  正文 自动生成（没有 publish/PR.md，建议手写一份说清能力用途）")

    print(f"  标题 {title}")
    print(f"  发布者 {publisher} | owner {owner} | 分支 {branch}")

    if args.dry_run:
        print("\n[dry-run] 将执行：")
        print(f"  1. 确保 fork {owner}/{args.upstream.split('/')[1]} 存在")
        print(f"  2. 克隆，基于 {args.upstream}/main 建分支 {branch}")
        print("  3. 提交 " + " + ".join(files))
        print(f"  4. 向 {args.upstream}:main 开 PR")
        return

    tok = token()
    fork_repo = args.upstream.split("/")[1]

    # 1. fork
    st, repo = api(tok, f"/repos/{owner}/{fork_repo}")
    if st == 404:
        print("创建 fork …")
        st, info = api(tok, f"/repos/{args.upstream}/forks", "POST", {"default_branch_only": True})
        if st not in (200, 201, 202):
            die(f"创建 fork 失败：{st} {info}")
        for i in range(40):
            time.sleep(3)
            st, repo = api(tok, f"/repos/{owner}/{fork_repo}")
            if st == 200:
                print(f"  fork 就绪（等了 {(i + 1) * 3} 秒）")
                break
        else:
            die("fork 迟迟没就绪，稍后重跑本脚本即可")
    else:
        print(f"  fork 已存在：{repo.get('full_name')}")

    # 2. 克隆
    fork_url = f"https://github.com/{owner}/{fork_repo}.git"
    if not (ROOT / ".git").exists():
        die("当前目录不是 git 仓库，先把插件项目推到 GitHub")
    print(f"克隆 {fork_url} …")
    for attempt in range(3):
        if (ROOT / ".tmp" / "registry").exists():
            shutil.rmtree(ROOT / ".tmp" / "registry")
        (ROOT / ".tmp").mkdir(parents=True, exist_ok=True)
        proc = subprocess.run(
            ["git", "clone", "--quiet", fork_url, str(ROOT / ".tmp" / "registry")],
            capture_output=True,
            text=True,
            env={**os.environ, "GIT_TERMINAL_PROMPT": "0"},
        )
        if proc.returncode == 0:
            break
        if attempt == 2:
            print((proc.stdout + proc.stderr).strip()[:800])
            die("克隆失败")
        print("  clone 失败，5 秒后重试")
        time.sleep(5)

    work = ROOT / ".tmp" / "registry"
    run(["git", "remote", "add", "upstream", f"https://github.com/{args.upstream}.git"], cwd=work)
    run(["git", "fetch", "--quiet", "upstream", "main"], cwd=work)
    run(["git", "checkout", "--quiet", "-B", branch, "upstream/main"], cwd=work)

    # 3. 放文件
    for rel, src in files.items():
        dst = work / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src, dst)
        print(f"  写入 {rel}")

    run(["git", "add"] + list(files), cwd=work)
    if not run(["git", "status", "--porcelain"], cwd=work).stdout.strip():
        die("没有差异：fork 里已经是这份内容了")

    message = f"Add {plugin_id}\n\nPublisher {publisher} and the {version} entry for {plugin_id}."
    run(
        [
            "git",
            "-c",
            f"user.name={owner}",
            "-c",
            f"user.email={owner}@users.noreply.github.com",
            "commit",
            "--quiet",
            "-m",
            message,
        ],
        cwd=work,
    )
    print("推送分支 …")
    run(["git", "push", "--quiet", "--force", "origin", branch], cwd=work)

    # 4. PR
    st, pr = api(
        tok,
        f"/repos/{args.upstream}/pulls",
        "POST",
        {"title": title, "head": f"{owner}:{branch}", "base": "main", "body": body},
    )
    if st in (200, 201):
        print(f"\nPR 已创建：{pr.get('html_url')}")
        return
    if st == 422:
        print("这个分支的 PR 可能已存在：")
        st2, pulls = api(tok, f"/repos/{args.upstream}/pulls?head={owner}:{branch}&state=all")
        if isinstance(pulls, list) and pulls:
            for p in pulls:
                print(f"  #{p['number']} {p['state']} {p['html_url']}")
            return
        die(f"422 {pr}")
    die(f"开 PR 失败：{st} {pr}")


if __name__ == "__main__":
    main()
