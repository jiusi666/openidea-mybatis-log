#!/usr/bin/env python3
"""OpenIDEA 插件的打包、签名与校验。

官方命令行 `openidea-plugin` 要在目标机器上从 IDE 源码编译（Rust），在 Windows
上还需要 MSVC 或完整的 MinGW 工具链。这里按官方插件包格式重新实现同样的
四个动作，任何有 Python 的机器都能复现发布包：

    keygen  生成发布者的 Ed25519 密钥（只在第一次做，之后保管好）
    check   按 IDE 安装时的规则检查一个目录或 .oiplugin 包
    pack    计算摘要、签名、写出 .oiplugin（zip）
    verify  解包重算摘要并验签，确认包没被改动过

格式与官方一致（已用 openidea 官方发布的包反证过）：

- 包是 zip（Deflate），条目为 `plugin.toml`、清单引用到的文件，最后是
  `SIGNATURE.json`；
- 摘要 = BLAKE3，对排序去重后的 (相对路径 + NUL + 文件内容 + NUL) 依次喂入，
  路径一律用 `/` 分隔，`plugin.toml` 也算在内；
- 签名 = Ed25519 over `openidea-plugin/1\\n<id>\\n<version>\\n<digest>`；
- `SIGNATURE.json` = {"publisher", "key", "signature"}，键与签名都是 base64。

依赖：`pip install blake3 cryptography`（tomllib 需要 Python 3.11+）。
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import re
import secrets
import sys
import tempfile
import tomllib
import zipfile
from pathlib import Path

try:
    from blake3 import blake3
except ImportError:  # pragma: no cover - 依赖缺失时说清楚
    sys.exit("缺少依赖：pip install blake3 cryptography")

try:
    from cryptography.hazmat.primitives.asymmetric.ed25519 import (
        Ed25519PrivateKey,
        Ed25519PublicKey,
    )
except ImportError:  # pragma: no cover
    sys.exit("缺少依赖：pip install blake3 cryptography")

MANIFEST = "plugin.toml"
SIGNATURE_FILE = "SIGNATURE.json"
EXTENSION = ".oiplugin"

# 市场允许的分类（最多 3 个）。
CATEGORIES = {
    "languages",
    "themes",
    "icon-themes",
    "linters",
    "vcs",
    "ai",
    "frameworks",
    "tools",
    "other",
}

PLUGIN_ID = re.compile(r"^[a-z0-9][a-z0-9_-]*(\.[a-z0-9][a-z0-9_-]*)+$")
PUBLISHER_ID = re.compile(r"^[a-z0-9][a-z0-9_-]{0,31}$")
SEMVER = re.compile(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$")

# 能力是权限：名字写错等于没申请。
# 按官方 docs/en/plugins/capabilities.mdx 的 Capability reference 对齐（API 1.0–1.5）：
# 旧版白名单漏了 editor.read / editor.write / clipboard / secrets 等，会让合法清单误报。
CAPABILITY = re.compile(
    r"^(storage|events|events\.input|clipboard|secrets|ide\.commands|"
    r"project\.(read|write)|editor\.(read|write)|vcs\.(read|write)|"
    r"database\.sources|ui\.[a-z._-]+|"
    r"(net|net\.http|net\.ask|process|fs\.read|fs\.write|ipc):[^\s]+)$"
)


def die(message: str) -> "NoReturn":  # noqa: F821 - 仅用于类型阅读
    sys.exit(f"错误：{message}")


def b64(raw: bytes) -> str:
    return base64.b64encode(raw).decode("ascii")


# --------------------------------------------------------------------------- 摘要


def hashed_path(rel: str) -> str:
    """包内相对路径统一成 `/` 分隔，Windows 与 Linux 得到同一个摘要。"""
    return rel.replace("\\", "/")


def package_digest(root: Path, files: list[str]) -> str:
    hasher = blake3()
    for rel in sorted({hashed_path(f) for f in files}):
        hasher.update(rel.encode("utf-8"))
        hasher.update(b"\x00")
        hasher.update((root / rel).read_bytes())
        hasher.update(b"\x00")
    return hasher.hexdigest()


def signed_message(plugin_id: str, version: str, digest: str) -> bytes:
    return f"openidea-plugin/1\n{plugin_id}\n{version}\n{digest}".encode("utf-8")


# --------------------------------------------------------------------------- 清单


def load_manifest(root: Path) -> dict:
    path = root / MANIFEST
    if not path.is_file():
        die(f"{path} 不存在")
    try:
        return tomllib.loads(path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as e:
        die(f"{MANIFEST} 不是合法 TOML：{e}")


def package_files(root: Path, manifest: dict) -> list[str]:
    """清单声明引用到的文件——IDE 安装时同样按这些字段收集。"""
    files: list[str] = []

    def add(value: object) -> None:
        if isinstance(value, str) and value and value not in files:
            files.append(hashed_path(value))

    add(manifest.get("wasm"))
    add(manifest.get("icon"))
    add(manifest.get("icon-dark"))
    for view in manifest.get("contributes", {}).get("views", []):
        add(view.get("web"))
        add(view.get("icon"))
        add(view.get("icon-dark"))
    settings = manifest.get("contributes", {}).get("settings")
    if isinstance(settings, dict):
        add(settings.get("schema"))
    if isinstance(settings, list):
        for item in settings:
            if isinstance(item, dict):
                add(item.get("schema"))
    missing = [f for f in files if not (root / f).is_file()]
    if missing:
        die("清单引用的文件不存在：" + "、".join(missing))
    return files


def capabilities(manifest: dict) -> tuple[list[str], list[str]]:
    caps = manifest.get("capabilities", {})
    required = caps.get("required", []) if isinstance(caps, dict) else []
    optional = caps.get("optional", []) if isinstance(caps, dict) else []
    return list(required), list(optional)


def check_manifest(root: Path, manifest: dict) -> tuple[str, str]:
    plugin_id = manifest.get("id", "")
    version = manifest.get("version", "")
    if not PLUGIN_ID.match(plugin_id):
        die(f"插件 id {plugin_id!r} 应为 <publisher>.<name>，只用小写字母、数字、'-' 和 '_'")
    if not SEMVER.match(version):
        die(f"版本 {version!r} 不是 semver（如 1.2.3）")
    for field in ("name", "description", "authors", "license"):
        if not manifest.get(field):
            die(f"{MANIFEST} 缺少 {field}（市场上会显示，也会被审阅）")
    if not manifest.get("engines", {}).get("openidea"):
        die(f"{MANIFEST} 缺少 engines.openidea（插件 API 版本要求）")

    cats = manifest.get("categories", [])
    if not cats:
        print("  ! 没有 categories，市场里只能归到 Uncategorized")
    elif len(cats) > 3:
        die(f"categories 最多 3 个，现在有 {len(cats)} 个")
    unknown = [c for c in cats if c not in CATEGORIES]
    if unknown:
        die("未知分类 " + "、".join(unknown) + "；可用：" + "、".join(sorted(CATEGORIES)))

    required, optional = capabilities(manifest)
    for cap in required + optional:
        if not CAPABILITY.match(str(cap)):
            die(f"能力 {cap!r} 不是已知能力（storage / net:<host> / process:<name> …）")

    wasm = manifest.get("wasm")
    if wasm:
        binary = root / wasm
        if not binary.is_file():
            die(f"wasm {wasm} 不存在，先构建")
        head = binary.read_bytes()[:8]
        if head[:4] != b"\x00asm":
            die(f"{wasm} 不是 WebAssembly 模块")
        if head[4:8] != b"\r\x00\x01\x00":
            die(f"{wasm} 不是 WebAssembly 组件（需要 wasm32-wasip2 + component）")
    return plugin_id, version


def packages(root: Path) -> tuple[list[str], str, str]:
    manifest = load_manifest(root)
    plugin_id, version = check_manifest(root, manifest)
    files = package_files(root, manifest)
    return files, plugin_id, version


# --------------------------------------------------------------------------- 动作


def cmd_keygen(args: argparse.Namespace) -> int:
    target = Path(args.file)
    if target.exists():
        die(f"{target} 已存在，不覆盖密钥")
    seed = secrets.token_bytes(32)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(b64(seed), encoding="utf-8")
    if os.name != "nt":
        os.chmod(target, 0o600)
    public = Ed25519PrivateKey.from_private_bytes(seed).public_key().public_bytes_raw()
    print(f"私钥：{target}（务必保密、备份；丢了就不能再更新自己的插件）")
    print(f"公钥：{b64(public)}")
    print("把这个公钥填进 openidea-plugins 的 publishers/<publisher>.toml。")
    return 0


def cmd_check(args: argparse.Namespace) -> int:
    target = Path(args.path)
    if target.is_file():
        with tempfile.TemporaryDirectory(prefix="oiplugin-check-") as tmp:
            manifest, files = unpack(target, Path(tmp))
            return report(Path(tmp), manifest, files)
    manifest = load_manifest(target)
    plugin_id, version = check_manifest(target, manifest)
    files = package_files(target, manifest)
    return report(target, manifest, files)


def report(root: Path, manifest: dict, files: list[str]) -> int:
    required, optional = capabilities(manifest)
    wasm = manifest.get("wasm")
    print(f"{manifest['id']} {manifest['version']} — {manifest['name']}")
    if wasm:
        size = (root / wasm).stat().st_size
        print(f"  code         {wasm} ({size / 1024:.0f} KB，component)")
    print(f"  categories   {', '.join(manifest.get('categories', [])) or '—'}")
    print(f"  api          {manifest.get('engines', {}).get('openidea')}")
    print(f"  required     {', '.join(required) or '—'}")
    print(f"  optional     {', '.join(optional) or '—'}")
    print(f"  files        {', '.join(files)}")
    print(f"  digest       {package_digest(root, files + [MANIFEST])}")
    print("ok")
    return 0


def cmd_pack(args: argparse.Namespace) -> int:
    root = Path(args.dir).resolve()
    files, plugin_id, version = packages(root)
    publisher = args.publisher
    if not PUBLISHER_ID.match(publisher):
        die(f"publisher {publisher!r} 只能用小写字母、数字、'-'、'_'（最多 32 位）")
    if not plugin_id.startswith(f"{publisher}."):
        die(f"publisher {publisher} 只能签 {publisher}.* 的插件，而这是 {plugin_id}")

    digest = package_digest(root, files + [MANIFEST])
    signature = None
    if args.key:
        seed = read_key(Path(args.key))
        key = Ed25519PrivateKey.from_private_bytes(seed)
        raw = key.sign(signed_message(plugin_id, version, digest))
        signature = {
            "publisher": publisher,
            "key": b64(key.public_key().public_bytes_raw()),
            "signature": b64(raw),
        }

    out = Path(args.out) if args.out else Path(f"{plugin_id}-{version}{EXTENSION}")
    names = sorted({*[hashed_path(f) for f in files], MANIFEST})
    out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as archive:
        for rel in names:
            info = zipfile.ZipInfo(rel, date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            archive.writestr(info, (root / rel).read_bytes())
        if signature:
            info = zipfile.ZipInfo(SIGNATURE_FILE, date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            archive.writestr(info, json.dumps(signature, indent=2, ensure_ascii=False))

    raw = out.read_bytes()
    print(f"-> {out} ({len(raw) / 1024:.0f} KB, "
          f"{'signed by ' + publisher if signature else 'unsigned'})")
    print(f"   digest  {digest}")
    print(f"   sha256  {hashlib.sha256(raw).hexdigest()}")
    print(f"   条目    {', '.join(zip_names(out))}")
    return 0


def read_key(path: Path) -> bytes:
    if not path.is_file():
        die(f"密钥文件 {path} 不存在（先跑 keygen）")
    try:
        raw = base64.b64decode(path.read_text(encoding="utf-8").strip(), validate=True)
    except ValueError as e:
        die(f"{path} 不是 base64：{e}")
    if len(raw) != 32:
        die(f"{path} 应为 32 字节的 Ed25519 种子（base64）")
    return raw


def zip_names(package: Path) -> list[str]:
    with zipfile.ZipFile(package) as archive:
        return [info.filename for info in archive.infolist() if not info.is_dir()]


def unpack(package: Path, dest: Path) -> tuple[dict, list[str]]:
    """解开一个包，返回 (清单, 参与摘要的文件)，规则与 IDE 一致。"""
    with zipfile.ZipFile(package) as archive:
        names = [n for n in archive.namelist() if not n.endswith("/")]
        prefix = ""
        if MANIFEST not in names:
            tops = {n.split("/", 1)[0] for n in names}
            if len(tops) == 1:
                prefix = f"{tops.pop()}/"
            if f"{prefix}{MANIFEST}" not in names:
                die(f"{package} 不是 OpenIDEA 插件包：根目录（或唯一子目录）里没有 {MANIFEST}")
        for name in names:
            if not name.startswith(prefix):
                continue
            rel = name[len(prefix):]
            parts = Path(rel).parts
            if not rel or rel.startswith("/") or ".." in parts:
                die(f"包内路径不合法：{name}")
            target = dest / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(archive.read(name))
    manifest = load_manifest(dest)
    files = [
        hashed_path(str(p.relative_to(dest)))
        for p in sorted(dest.rglob("*"))
        if p.is_file() and p.name not in (MANIFEST, SIGNATURE_FILE)
    ]
    return manifest, files


def cmd_verify(args: argparse.Namespace) -> int:
    package = Path(args.package)
    with tempfile.TemporaryDirectory(prefix="oiplugin-verify-") as tmp:
        dest = Path(tmp)
        manifest, files = unpack(package, dest)
        plugin_id, version = manifest.get("id", ""), manifest.get("version", "")
        sig_path = dest / SIGNATURE_FILE
        if not sig_path.is_file():
            die("包没有签名（SIGNATURE.json），市场只收签名包")
        signature = json.loads(sig_path.read_text(encoding="utf-8"))
        digest = package_digest(dest, files + [MANIFEST])
        try:
            public = Ed25519PublicKey.from_public_bytes(base64.b64decode(signature["key"]))
            public.verify(
                base64.b64decode(signature["signature"]),
                signed_message(plugin_id, version, digest),
            )
        except Exception:
            die("签名与该包内容不匹配（包被改动过，或 id/版本与签名时不同）")
        if not plugin_id.startswith(f"{signature['publisher']}."):
            die(f"publisher {signature['publisher']} 不能发布 {plugin_id}")
        print(f"ok  {plugin_id} {version}")
        print(f"    publisher {signature['publisher']}")
        print(f"    digest    {digest}")
        print(f"    sha256    {hashlib.sha256(package.read_bytes()).hexdigest()}")
        print(f"    文件      {', '.join(files)}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)

    k = sub.add_parser("keygen", help="生成发布者的 Ed25519 密钥")
    k.add_argument("file")
    k.set_defaults(run=cmd_keygen)

    c = sub.add_parser("check", help="检查目录或 .oiplugin 包")
    c.add_argument("path", nargs="?", default=".")
    c.set_defaults(run=cmd_check)

    p = sub.add_parser("pack", help="打包（可签名）")
    p.add_argument("dir", nargs="?", default=".")
    p.add_argument("--key", help="发布者私钥（与 --publisher 一起用）")
    p.add_argument("--publisher", help="发布者 id，必须是插件 id 的前缀")
    p.add_argument("--out", help="输出文件，默认 <id>-<version>.oiplugin")
    p.set_defaults(run=cmd_pack)

    v = sub.add_parser("verify", help="解包重算摘要并验签")
    v.add_argument("package")
    v.set_defaults(run=cmd_verify)

    args = parser.parse_args()
    if args.command == "pack" and bool(args.key) != bool(args.publisher):
        die("--key 和 --publisher 要一起给")
    return args.run(args)


if __name__ == "__main__":
    sys.exit(main())
