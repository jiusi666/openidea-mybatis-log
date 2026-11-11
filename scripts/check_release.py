#!/usr/bin/env python3
"""发布后自检：确认 Release 附件与 registry 条目一致。

在插件项目根目录运行：

    python scripts/check_release.py            # 下载远端附件核对
    python scripts/check_release.py --local    # 只核对本地 dist/ 下的包

读 `plugin.toml` 得到 id 与版本，读 `publish/plugins/<id>.toml` 得到
package 地址与 sha256，然后依次确认：

    1. 地址能下到文件（占位或 404 会直接报错）；
    2. 下载文件的 sha256 与条目里的相同；
    3. 包能通过摘要重算与签名验证（调 oiplugin.py verify）；
    4. 远端附件与本地 dist/ 是同一份字节。

任何一项不通过都以非零状态退出，提 PR 前跑一遍。
"""

from __future__ import annotations

import argparse
import hashlib
import os
import subprocess
import sys
import tempfile
import tomllib
import urllib.error
import urllib.request
from pathlib import Path

# Windows 控制台默认 GBK：中文会乱码、勾叉直接抛 UnicodeEncodeError。
# 这里把两个流钉成 UTF-8；标记只用 ASCII，编码再退化也不会中断。
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8", errors="replace")
    except (AttributeError, ValueError):
        pass

def _root() -> Path:
    """项目根：优先当前目录（在项目里跑），否则脚本所在项目（脚本被拷进 scripts/ 时）。"""
    cwd = Path.cwd()
    if (cwd / "plugin.toml").is_file():
        return cwd
    return Path(__file__).resolve().parent.parent


ROOT = _root()
TOOL = Path(__file__).resolve().parent / "oiplugin.py"


def fail(msg: str) -> None:
    print(f"  [x] {msg}")
    sys.exit(1)


def project() -> tuple[str, str]:
    path = ROOT / "plugin.toml"
    if not path.is_file():
        fail(f"找不到 {path}，请在插件项目根目录运行")
    manifest = tomllib.loads(path.read_text(encoding="utf-8"))
    return manifest["id"], manifest["version"]


def fetch(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "openidea-plugin-check"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        return resp.read()


def verify_package(blob: bytes) -> tuple[bool, list[str]]:
    """把包写到临时文件，交给 oiplugin.py 重算摘要并验签。"""
    with tempfile.TemporaryDirectory() as tmp:
        pkg = Path(tmp) / "pkg.oiplugin"
        pkg.write_bytes(blob)
        # 子进程统一走 UTF-8：否则 Windows 下它会按 GBK 写、这里按 UTF-8 读，
        # 解码异常在线程里被吞掉，验证信息就静默变空了。
        proc = subprocess.run(
            [sys.executable, str(TOOL), "verify", str(pkg)],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            env={**os.environ, "PYTHONIOENCODING": "utf-8"},
        )
    out = (proc.stdout or "") + (proc.stderr or "")
    lines = [ln.strip() for ln in out.splitlines() if ln.strip()]
    head = next((ln for ln in lines if ln.startswith("ok ")), "")
    digest = next((ln for ln in lines if ln.startswith("digest")), "")
    picked = [ln for ln in (head, digest) if ln]
    return proc.returncode == 0, picked or [out.strip()]


def main() -> None:
    parser = argparse.ArgumentParser(description="核对发布包与 registry 条目")
    parser.add_argument("--local", action="store_true", help="不联网，只核对本地 dist/ 下的包")
    parser.add_argument("--version", help="指定版本（默认取 plugin.toml 里的）")
    args = parser.parse_args()

    plugin_id, manifest_version = project()
    version = args.version or manifest_version
    entry_path = ROOT / "publish" / "plugins" / f"{plugin_id}.toml"
    if not entry_path.is_file():
        fail(f"找不到 registry 条目 {entry_path}")

    entry = tomllib.loads(entry_path.read_text(encoding="utf-8"))
    versions = entry.get("versions") or []
    hit = next((v for v in versions if v.get("version") == version), None)
    if hit is None:
        known = ", ".join(v.get("version", "?") for v in versions) or "（空）"
        fail(f"条目里没有 {version} 这一版；已有：{known}")

    url = hit.get("package", "")
    want = (hit.get("sha256") or "").lower()
    local = ROOT / "dist" / f"{plugin_id}-{version}.oiplugin"

    print(f"{plugin_id} {version}")
    print(f"  地址 {url}")

    if args.local:
        if not local.is_file():
            fail(f"本地缺少 {local}")
        blob = local.read_bytes()
        print("  来源 本地 dist/")
    else:
        try:
            blob = fetch(url)
        except urllib.error.HTTPError as exc:
            fail(f"下载失败：HTTP {exc.code}（Release 还没发，或文件名不是 {local.name}）")
        except Exception as exc:  # noqa: BLE001 - 网络问题种类很多，统一报出来
            fail(f"下载失败：{exc}")
        print(f"  来源 远端（{len(blob)} 字节）")

    got = hashlib.sha256(blob).hexdigest()
    ok = True
    if got != want:
        ok = False
        print(f"  [x]  sha256 不一致\n      条目 {want}\n      实际 {got}")
    else:
        print(f"  [ok] sha256 {got}")

    passed, detail = verify_package(blob)
    if not passed:
        ok = False
        print("  [x]  包校验失败")
        for line in detail:
            print(f"      {line}")
    else:
        for line in detail:
            print(f"  [ok] {line}")

    if not args.local and local.is_file():
        if local.read_bytes() != blob:
            ok = False
            print("  [x]  远端附件与本地 dist/ 不是同一份字节")
        else:
            print("  [ok] 与本地 dist/ 一致")

    print()
    print("全部通过，可以去提 PR。" if ok else "存在问题，先别提交。")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
