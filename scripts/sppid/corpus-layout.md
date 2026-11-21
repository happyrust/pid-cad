# 阶段一语料目录约定与清单规范（`corpus-layout`）

> 对应任务：[happyrust/pid-cad#107](https://github.com/happyrust/pid-cad/issues/107)（`L0-2`）。
> 配套工具：`scripts/sppid/corpus_index.py`（纯 Python 标准库）。

---

## 1. 设计目标

1. **报告数字与具体样本文件强绑定**：每份度量或校验报告均记录 `corpus.manifest.json` 的 `manifest_sha256` 摘要，保证任何时刻能核实报告对应的精确文件集合。
2. **零客户数据、零绝对路径入库**：客户图纸与备份样本放在仓库外，由环境变量 `OCS_PID_CORPUS_DIR` 指向；生成的 `corpus.manifest.json` 内只记录正斜杠（`/`）相对路径、字节数与 `SHA-256`，严禁出现盘符、根路径、反斜杠或 `..` 越界段。
3. **无样本环境零误伤门禁**：当未传 `--dir` 且未设置 `OCS_PID_CORPUS_DIR` 时，`corpus_index.py` 打印 `未设置语料目录` 并以退出码 `0` 正常跳过。

---

## 2. 默认逻辑名与相对子目录布局

| 逻辑名 | 默认相对子目录 | 存放内容 |
|---|---|---|
| `EXPORT_SET` | `export-set` | SPPID 导出的成对样本（如 `export-set/<批次>/` 下同名的 `<图名>.pid`、`<图名>_Data.xml`、`<图名>_Meta.xml`） |
| `EXPORT_DXF` | `export-dxf` | SPPID 对同批图导出的参考 DXF |
| `EXPORT_PDF` | `export-pdf` | SPPID 对同批图导出的参考 PDF |
| `SOURCE_DWG` | `source-dwg` | 与 `EXPORT_SET` 对应的存量原始 DWG / DXF |
| `BACKUP_ORACLE` | `backup-oracle` | Oracle 导出形式的备份包（目录或 zip） |
| `BACKUP_TABLES` | `backup-tables` | 带数据库表文本导出的备份包（目录或 zip） |
| `SYMBOLS` | `symbols` | `.sym` 符号库与 `.igr` 图框模板 |
| `SEEDS` | `seeds` | 空白或受控种子 `.pid` 文件及模板说明 |
| `EXTERNAL` | `external-platform` | 外部交付平台的规范文档、已通过清单与回执说明 |
| `CONTROLLED` | `controlled` | 单一原子编辑前后的 `.pid` 受控对照样本对 |

### 自定义映射（不搬移原目录文件）

当本机已有样本目录未按上表默认名字摆放时，**无需移动或复制原始样本文件**，生成清单时直接通过 `--map <逻辑名>=<相对子目录>` 指定相对路径映射：

```powershell
python scripts/sppid/corpus_index.py index `
  --dir $env:OCS_PID_CORPUS_DIR `
  --manifest $env:OCS_PID_EVIDENCE_DIR\corpus.manifest.json `
  --map EXPORT_SET=publish/sppid-export/pid-and-xml `
  --map SYMBOLS=Ref/Symbols
```

- 指定 `--manifest` 将清单写到外部证据目录时，语料源目录保持 **100% 只读零写入**。
- 再次对同一 `--manifest` 执行 `index` 时，若未传 `--reset-layout`，工具会自动保留清单里已登记的自定义相对子目录映射；也可新增自定义逻辑名（如 `VENDOR_REF=...`）以区分多套样本分母。

---

## 3. `corpus.manifest.json` 结构

```json
{
  "schema_version": 1,
  "generated_at": "2026-11-20T15:00:00Z",
  "manifest_sha256": "<64位小写十六进制：基于 schema_version + sections 规范化 JSON 计算>",
  "total_files": 4,
  "total_bytes": 123456,
  "sections": {
    "EXPORT_SET": {
      "subdir": "export-set",
      "present": true,
      "file_count": 3,
      "total_bytes": 120000,
      "files": [
        {
          "path": "batch01/sample.pid",
          "bytes": 98000,
          "sha256": "<64位小写十六进制>"
        }
      ]
    }
  }
}
```

- `manifest_sha256` 不受 `generated_at` 时间戳影响：同一批文件重复执行 `index`，`manifest_sha256` 恒定不变。

---

## 4. 命令用法与退出码

```powershell
# 1. 生成语料清单
python scripts/sppid/corpus_index.py index [--dir <语料根>] [--manifest <清单路径>] [--map KEY=REL_SUBDIR ...]

# 2. 校验语料目录是否与清单逐字节一致
python scripts/sppid/corpus_index.py verify [--dir <语料根>] [--manifest <清单路径>]
```

| 退出码 | 含义 |
|---:|---|
| `0` | 生成成功 / 校验全部一致 / 未设 `OCS_PID_CORPUS_DIR` 且未传 `--dir` 自动跳过 |
| `1` | `verify` 发现文件缺失（`MISSING`）、未登记的新增文件（`UNEXPECTED`）、字节或哈希被改动（`MISMATCH`）或清单摘要不一致 |
| `2` | 命令行参数错误、越界/绝对路径映射、语料目录或清单文件不存在 |
