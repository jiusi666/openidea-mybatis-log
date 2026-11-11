#!/usr/bin/env python3
"""看清 OpenIDEA 里这个插件是怎么装的，并安全地解除本地安装。

    python scripts/local_install.py status                 # 只读：列出四种安装痕迹
    python scripts/local_install.py clean                  # 试算要改什么，不写文件
    python scripts/local_install.py clean --apply          # 真的解除（先备份）
    python scripts/local_install.py clean --apply --purge-data   # 连本地存储一起删
    python scripts/local_install.py link --apply           # 把当前目录挂成开发插件

OpenIDEA 的插件有三种来源（宿主自己的说法）：

    Installed  <app-data>/plugins/<id>/        从 .oiplugin 装进来的，有实体目录
    Dev        <project>/.openidea/plugins/<d>/ 项目内的开发插件，随项目打开而加载
    Linked     由 links.json 记住的任意目录      「Install from Disk」选目录就是这个

**Linked 没有"卸载"可言**：删掉 `plugins/<id>/` 不起作用，因为那里根本没有东西；
要移除只能把路径从 links.json 里去掉（宿主自己的报错也是这么说的：
"development plugins are removed by deleting them ... or unlinking them"）。

这个脚本只动与本插件相关的条目：links.json 里那个路径、state.json 里那条 disabled、
grants.json 里 kind=path 的那一条。改之前把原文件备份成 `*.bak-<时间戳>`，
IDE 正在运行时直接拒绝——它退出时会把内存里的状态写回去，覆盖掉你的修改。
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
import tomllib
from pathlib import Path

for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8", errors="replace")
    except (AttributeError, ValueError):
        pass

APP_DIR_NAME = "ai.l0ng.openidea"


def die(msg: str) -> None:
    print(f"  [x] {msg}")
    sys.exit(1)


def app_data() -> Path:
    override = os.environ.get("OPENIDEA_APP_DATA")
    if override:
        return Path(override).expanduser()
    home = Path.home()
    if sys.platform.startswith("win"):
        base = Path(os.environ.get("APPDATA", home / "AppData" / "Roaming"))
        return base / APP_DIR_NAME
    if sys.platform == "darwin":
        return home / "Library" / "Application Support" / APP_DIR_NAME
    for cand in (home / ".local" / "share" / APP_DIR_NAME, home / ".config" / APP_DIR_NAME):
        if cand.is_dir():
            return cand
    return home / ".local" / "share" / APP_DIR_NAME


def ide_running() -> list[str]:
    try:
        if sys.platform.startswith("win"):
            out = subprocess.run(
                ["tasklist", "/FI", "IMAGENAME eq openidea.exe"],
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
            ).stdout
            return ["openidea.exe"] if "openidea.exe" in out.lower() else []
        out = subprocess.run(["pgrep", "-fl", "openidea"], capture_output=True, text=True).stdout
        return [ln for ln in out.splitlines() if ln.strip()]
    except Exception:
        return []


def load_json(path: Path, default):
    if not path.is_file():
        return default
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        die(f"{path} 不是合法 JSON：{exc}（先手工修好，别让脚本覆盖它）")


def save_json(path: Path, data, stamp: str, apply: bool) -> bool:
    text = json.dumps(data, ensure_ascii=False, indent=2) + "\n"
    if not apply:
        return True
    if path.is_file():
        backup = path.with_name(f"{path.name}.bak-{stamp}")
        shutil.copyfile(path, backup)
        print(f"      备份 {backup.name}")
    path.write_text(text, encoding="utf-8", newline="\n")
    return True


def project_info(project: Path) -> tuple[str, str, str]:
    manifest = project / "plugin.toml"
    if not manifest.is_file():
        die(f"{project} 下没有 plugin.toml，用 --project 指到插件项目根目录")
    data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    return data["id"], data["version"], data.get("name", data["id"])


def dir_size(path: Path) -> int:
    if not path.exists():
        return 0
    return sum(f.stat().st_size for f in path.rglob("*") if f.is_file())


def cmd_status(project: Path) -> None:
    plugin_id, version, name = project_info(project)
    root = app_data()
    plugins = root / "plugins"

    print(f"插件   {name}  ({plugin_id} {version})")
    print(f"源码   {project}")
    print(f"数据目录 {root}")

    running = ide_running()
    if running:
        print("状态   OpenIDEA 正在运行 —— 改文件前请先退出，否则退出时会被覆盖")
    else:
        print("状态   未检测到 OpenIDEA 进程")

    if not root.is_dir():
        print("\n没找到 OpenIDEA 数据目录（还没运行过 IDE？）")
        return

    # 1. Installed
    installed = plugins / plugin_id
    print("\n[Installed] 安装包方式")
    if installed.is_dir():
        if (installed / "plugin.toml").is_file():
            got = tomllib.loads((installed / "plugin.toml").read_text(encoding="utf-8"))
            print(f"  有目录 {installed}")
            print(f"  目录内版本 {got.get('version')}（{dir_size(installed) // 1024} KB）")
        else:
            print(f"  有目录 {installed}（里面没有 plugin.toml，可能是残留）")
        print("  移除：IDE 里 Settings › Plugins 卸载；或退出 IDE 后删掉这个目录")
    else:
        print("  无")

    # 2. Linked
    links = load_json(plugins / "links.json", {"paths": []})
    paths = links.get("paths") or []
    mine = [p for p in paths if Path(p).resolve() == project.resolve()]
    print("\n[Linked] 目录链接方式")
    if paths:
        for p in paths:
            mark = "  ← 就是它" if p in mine else ""
            print(f"  {p}{mark}")
    else:
        print("  无")
    if mine:
        print("  移除：没有卸载按钮，把这条路径从 links.json 去掉（本脚本 clean 会做）")

    # 3. Dev
    dev = project / ".openidea" / "plugins"
    print("\n[Dev] 项目内开发插件")
    if dev.is_dir() and any(dev.iterdir()):
        print(f"  存在 {dev}")
        for d in sorted(dev.iterdir()):
            print(f"    {d.name}")
        print("  移除：直接删掉 .openidea/plugins 下那个目录")
    else:
        print("  无")

    # 4. 状态与授权
    state = load_json(plugins / "state.json", {})
    disabled = state.get("disabled") or []
    print("\n[state] 启用状态")
    print(f"  disabled: {disabled or '（空）'}" + ("   ← 本插件被禁用" if plugin_id in disabled else ""))

    grants = load_json(plugins / "grants.json", {"grants": []}).get("grants") or []
    print("\n[grants] 用户已授权的能力")
    hit = []
    for entry in grants:
        if not isinstance(entry, list) or len(entry) != 2:
            continue
        key, grant = entry
        if not isinstance(grant, dict):
            continue
        same_id = grant.get("pluginId") == plugin_id
        same_path = key.get("kind") == "path" and Path(str(key.get("value"))).resolve() == project.resolve()
        if same_id or same_path:
            hit.append((key, grant))
            caps = ", ".join(grant.get("granted") or []) or "（无）"
            print(f"  {key.get('kind')}={key.get('value')}  id={grant.get('pluginId')} v{grant.get('version')}")
            print(f"    能力 {caps}")
    if not hit:
        print("  无本插件的记录")

    # 5. 本地存储
    data_dir = plugins / ".data" / plugin_id
    print("\n[.data] 插件本地存储（设置、缓存）")
    print(f"  {data_dir}  {dir_size(data_dir) // 1024} KB" if data_dir.is_dir() else "  无")

    print("\n结论")
    if mine:
        print("  这个插件是以「链接源码目录」的方式装着的：改代码重新构建就生效，没有卸载入口。")
        print("  要移除：python scripts/local_install.py clean --apply（先退出 OpenIDEA）")
    elif installed.is_dir():
        print("  已作为安装包装入。要移除：Settings › Plugins 卸载，或退出后删目录。")
    else:
        print("  没发现这个插件的安装痕迹。")


def cmd_clean(project: Path, apply: bool, purge_data: bool) -> None:
    plugin_id, version, _ = project_info(project)
    root = app_data()
    plugins = root / "plugins"
    if not plugins.is_dir():
        die(f"没有 {plugins}")

    if apply and ide_running():
        die("OpenIDEA 还在运行。先退出（它会把手里的状态写回去），再执行 --apply")

    stamp = time.strftime("%Y%m%d-%H%M%S")
    ops: list[str] = []

    # links.json
    links_path = plugins / "links.json"
    links = load_json(links_path, {"paths": []})
    paths = links.get("paths") or []
    keep = [p for p in paths if Path(p).resolve() != project.resolve()]
    if len(keep) != len(paths):
        links["paths"] = keep
        ops.append(f"links.json: 去掉 {project}")
        if apply:
            save_json(links_path, links, stamp, apply)

    # 顺带清理指向不存在目录的条目（只报告，不动）
    stale = [p for p in keep if not Path(p).exists()]
    for p in stale:
        print(f"  ! links.json 里还有个失效路径，你可以顺手清掉：{p}")

    # state.json
    state_path = plugins / "state.json"
    state = load_json(state_path, {})
    disabled = state.get("disabled") or []
    if plugin_id in disabled:
        state["disabled"] = [d for d in disabled if d != plugin_id]
        ops.append(f"state.json: 从 disabled 去掉 {plugin_id}")
        if apply:
            save_json(state_path, state, stamp, apply)

    # grants.json
    grants_path = plugins / "grants.json"
    grants_doc = load_json(grants_path, {"grants": []})
    grants = grants_doc.get("grants") or []
    kept = []
    dropped = 0
    for entry in grants:
        if isinstance(entry, list) and len(entry) == 2 and isinstance(entry[0], dict) and isinstance(entry[1], dict):
            key, grant = entry
            same_path = key.get("kind") == "path" and Path(str(key.get("value"))).resolve() == project.resolve()
            same_id_path = grant.get("pluginId") == plugin_id and key.get("kind") == "path"
            if same_path or same_id_path:
                dropped += 1
                continue
        kept.append(entry)
    if dropped:
        grants_doc["grants"] = kept
        ops.append(f"grants.json: 删掉 {dropped} 条 path 授权记录")
        if apply:
            save_json(grants_path, grants_doc, stamp, apply)

    # 本地存储
    data_dir = plugins / ".data" / plugin_id
    if purge_data and data_dir.is_dir():
        ops.append(f".data/{plugin_id}: 删除（{dir_size(data_dir) // 1024} KB 的设置与缓存）")
        if apply:
            shutil.rmtree(data_dir)
    elif data_dir.is_dir():
        print(f"  · 保留 .data/{plugin_id}（{dir_size(data_dir) // 1024} KB）；要一起删加 --purge-data")

    # 安装包目录（链接式插件正常不会有，有就是残留）
    installed = plugins / plugin_id
    if installed.is_dir():
        print(f"  · 发现安装目录 {installed}，本脚本不动它：先确认它是不是从市场装的")

    print()
    if not ops:
        print("没有需要改的：这个插件没有链接、禁用或授权记录。")
        return
    print("将执行：" if not apply else "已执行：")
    for op in ops:
        print(f"  - {op}")
    if not apply:
        print("\n这是试算。加 --apply 真正写入（会先备份成 *.bak-<时间戳>）。")
        print("记得先退出 OpenIDEA。")
        return
    print("\n完成。重新打开 OpenIDEA，Settings › Plugins 里就不再有它了。")
    print("要装市场版：Settings › Plugins › Available 搜插件名；要装本地包：Install from Disk 选 .oiplugin。")


def cmd_link(project: Path, apply: bool) -> None:
    plugin_id, version, _ = project_info(project)
    root = app_data()
    plugins = root / "plugins"
    plugins.mkdir(parents=True, exist_ok=True)
    links_path = plugins / "links.json"
    links = load_json(links_path, {"paths": []})
    paths = links.get("paths") or []
    target = str(project)

    if any(Path(p).resolve() == project.resolve() for p in paths):
        print(f"已经链接过了：{target}")
        return
    others = [p for p in paths if Path(p).exists()]

    print(f"把 {target} 挂成开发插件（{plugin_id} {version}）")
    print(f"  links.json 现在有 {len(paths)} 条，其中 {len(others)} 条路径有效")
    if not apply:
        print("\n试算。加 --apply 写入（IDE 需先退出）。")
        return
    if ide_running():
        die("OpenIDEA 还在运行，先退出")
    links["paths"] = paths + [target]
    save_json(links_path, links, time.strftime("%Y%m%d-%H%M%S"), apply)
    print("完成。打开 IDE 后插件会从源码目录加载；改了 Rust 要重新构建 wasm。")


def main() -> None:
    ap = argparse.ArgumentParser(description="查看/解除 OpenIDEA 插件的本地安装")
    ap.add_argument("command", choices=["status", "clean", "link"], nargs="?", default="status")
    ap.add_argument("--project", help="插件项目根目录（默认当前目录）")
    ap.add_argument("--apply", action="store_true", help="真的写文件（默认只试算）")
    ap.add_argument("--purge-data", action="store_true", help="连 .data 下的本地存储一起删")
    args = ap.parse_args()

    project = Path(args.project).expanduser().resolve() if args.project else Path.cwd().resolve()

    if args.command == "status":
        cmd_status(project)
    elif args.command == "clean":
        cmd_clean(project, args.apply, args.purge_data)
    else:
        cmd_link(project, args.apply)


if __name__ == "__main__":
    main()
