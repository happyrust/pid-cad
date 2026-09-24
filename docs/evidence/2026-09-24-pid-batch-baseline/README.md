# `.pid` 批量回归基线（2026-09-24，计划 `2026-09-24-pid-import-status-and-next-steps.md` B1）

`report.csv` 是 `examples/pid_batch_report.rs` 在 pid-parse `test-file/` 上跑出来的一份：六张 `.pid`（四张主图、publish 目录的 A01 与
DWG-0202 副本），每张一行——`load_pid` 的导入摘要（实体 / 记录 / 没画 / 缓存与库本体 / 关闭层笔画 / 图纸图层 / 压平文字 / 样式表是否读出）、
同一次解析的拒收与无解码器记录按类型码计数、单位、页幅，以及照 `--export` 的路另存 DXF 的 SHA-256。

- 基线：OCS `72ae8f42` + pid-parse `686c9d5`（`DependencyObject` 按成员数校验之后，0201 的 `missing` 为 0），debug 构建。四张主图的 `dxf_sha256` 与计划里 T2 的新基线逐一相同；
  publish 目录那张 DWG-0202 旁边带 `_Data.xml`，语义进了 XDATA，所以哈希与 `test-file/` 那张不同。
- `import_ms` 只作参考，随机器与负载变。

重跑与对比：

```powershell
cargo run --example pid_batch_report -- ..\pid-parse\test-file --export-dir $env:TEMP\pid-batch > report-new.csv
Compare-Object (Import-Csv docs\evidence\2026-09-24-pid-batch-baseline\report.csv | Select-Object * -ExcludeProperty import_ms) `
               (Import-Csv report-new.csv | Select-Object * -ExcludeProperty import_ms) -Property file, drawn, decoded, missing, refused, undecoded, dxf_sha256
```

新语料来了（B2）：把目录直接交给它，`status` 不是 `ok` 的行、`refused` / `undecoded` 里出现新类型码的行、`unit` 为 `assumed-metre` 或 `page_mm` 为 `-` 的行，
就是下一批要开的单。
