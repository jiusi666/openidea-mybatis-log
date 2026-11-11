# SDK snapshot

`assets/sdk/` 是 OpenIDEA 官方 Rust SDK 的源码快照，用来让新项目离线可构建、
且每次构建得到同一份 wasm。

- 上游：https://github.com/l0ng-ai/openidea （`ide-host` 工作区里的 `openidea-plugin` crate）
- 版本：v0.1.40，对应插件 API `openidea:plugin@1.4.0`（清单里写 `engines.openidea = "^1.4"`）
- 许可：Apache-2.0，见上游仓库的 LICENSE
- 改动：删除了 `tests/`（SDK 自身的测试，构建用不到）

更新方式：把 `vendor/openidea-plugin` 换成新 tag 的源码，同时把
`Cargo.toml` 里的 git tag 与清单的 `engines.openidea` 一起改掉，然后重跑
`cargo test` 与 `oiplugin.py check`。
