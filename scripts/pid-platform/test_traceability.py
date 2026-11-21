"""The ``-k traceability`` gate: requirement IDs, area files and the matrix.

Every test name starts with ``test_traceability_`` so ``pytest -k traceability``
selects this module and nothing else. The validators live in
``requirements_contract.py``; the negative controls below prove each of the six
failure classes is actually detected, not merely described.
"""
from __future__ import annotations

import json

import pytest

import conftest
import rule_dsl
from conftest import CATALOG, DOCS, PROTO, REPO_ROOT, SCREEN_IDS
from docs_contract import find_table
from manifest_contract import normalize_heading
from requirements_contract import (
    AREA_FLOOR,
    COLLABORATION_SECTIONS,
    CONNECTION_POINT_DIRECTIONS,
    CONNECTION_POINT_TYPES,
    DATA_SECTIONS,
    DELIVERABLE_SECTIONS,
    DRAWING_SECTIONS,
    ERROR_TABLE_COLUMNS,
    ERROR_TABLE_SECTION,
    IMPORT_EXPORT_SECTIONS,
    MATRIX_COLUMNS,
    MIGRATION_SECTIONS,
    OBSERVABLE_WORDS,
    OPEN_COLUMNS,
    OPEN_SECTION,
    OVERVIEW_PATH,
    PROTOCOL_SOURCE,
    REQ_AREA_FILES,
    REQ_ID_RE,
    REQ_ID_TEXT,
    REUSE_MAPPING_SECTIONS,
    RULE_TABLE_COLUMNS,
    SHIPPED_DESIGN_RULES,
    STATUS_COLUMNS,
    STATUS_SECTION,
    STATUS_VALUES,
    SYMBOL_SECTIONS,
    TAG_RULE_FIELDS,
    TYPE_SECTIONS,
    PLATFORM_SECTIONS,
    _plain_cell,
    bom_rows,
    definitions,
    ebnf_productions,
    error_table_codes,
    find_bom_header,
    load_all_documents,
    load_documents,
    matrix_rows,
    protocol_error_codes,
    referenced_error_codes,
    section_requirement_counts,
    section_text,
    split_entries,
    strip_list_number,
    undefined_ebnf_nonterminals,
    validate_area_floors,
    validate_area_table,
    validate_bom_row_identity,
    validate_bom_rows,
    validate_collaboration_coverage,
    validate_collaboration_error_table,
    validate_collaboration_open_questions,
    validate_collaboration_status,
    validate_connection_point_table,
    validate_data_rule_engine_coverage,
    validate_deliverable_coverage,
    validate_design_rule_table,
    validate_dr004_independence,
    validate_drawing_coverage,
    validate_import_export_coverage,
    validate_matrix,
    validate_migration_coverage,
    validate_platform_coverage,
    validate_requirement_ids,
    validate_entries,
    validate_reuse_mapping,
    validate_result_model,
    validate_rule_dsl_grammar,
    validate_symbol_library_coverage,
    validate_tag_master_key,
    validate_traceability,
    validate_trigger_modes,
    validate_type_library_coverage,
    whitelisted_sections,
)

PLAT_PATH = "requirements/10-platform.md"
TYPE_PATH = "requirements/20-type-library.md"
SYMBOL_PATH = "requirements/30-symbol-library-and-project-settings.md"
DRAWING_PATH = "requirements/40-drawing-and-drafting.md"
DATA_PATH = "requirements/50-data-and-rule-engine.md"
DELIVERABLE_PATH = "requirements/60-deliverables-and-publish.md"
COLLABORATION_PATH = "requirements/70-collaboration-and-permissions.md"
IMPORT_EXPORT_PATH = "requirements/80-import-export-matrix.md"
MIGRATION_PATH = "requirements/90-migration-and-interop.md"
REUSE_PATH = "requirements/95-reuse-mapping.md"

# Every declared section, paired with the area document that must carry it as
# an H2 heading. The names themselves live in the contract module; the pairing
# is what makes the parametrized check below assert file state.
SECTION_HEADINGS = (
    [(PLAT_PATH, name) for name in PLATFORM_SECTIONS]
    + [(TYPE_PATH, name) for name in TYPE_SECTIONS]
    + [(SYMBOL_PATH, name) for name in SYMBOL_SECTIONS]
    + [(DRAWING_PATH, name) for name in DRAWING_SECTIONS]
    + [(DATA_PATH, name) for name in DATA_SECTIONS]
    + [(DELIVERABLE_PATH, name) for name in DELIVERABLE_SECTIONS]
    + [(COLLABORATION_PATH, name) for name in COLLABORATION_SECTIONS]
    + [(IMPORT_EXPORT_PATH, name) for name in IMPORT_EXPORT_SECTIONS]
    + [(MIGRATION_PATH, name) for name in MIGRATION_SECTIONS]
    + [(REUSE_PATH, name) for name in REUSE_MAPPING_SECTIONS]
)


def _screen_ids() -> list:
    """The 14 prototype screen ids, from the catalog once it exists."""
    if CATALOG.exists():
        with CATALOG.open(encoding="utf-8") as handle:
            catalog = json.load(handle)
        return [screen["id"] for screen in catalog.get("screens", [])]
    return list(SCREEN_IDS)


def _entry(req_id: str, source: str = "§4.1.1", title: str = "示例需求") -> str:
    return (
        "### %s %s\n\n"
        "- **优先级**：必须\n"
        "- **用户故事**：作为设计人员，我希望执行示例能力，以便验证契约。\n"
        "- **验收标准**：\n"
        "  1. 返回 `OK` 且计数为 1。\n"
        "  2. 列表行数增加 1。\n"
        "- **数据与界面契约**：\n"
        "  | 字段 | 类型 | 必填 | 约束 |\n"
        "  | --- | --- | --- | --- |\n"
        "  | 示例字段 | String | 是 | ≤ 64 字符 |\n"
        "- **来源**：%s\n"
        % (req_id, title, source)
    )


def _vague_entry(req_id: str) -> str:
    """An entry whose criteria are pure sentiment, each with a different list marker.

    ``1.`` / ``2、`` / ``3)`` are the three markers the contract accepts; all of
    them must be stripped before decidability is judged.
    """
    return (
        "### %s 情感描述不可判定\n\n"
        "- **优先级**：必须\n"
        "- **用户故事**：作为设计人员，我希望系统好用，以便提升效率。\n"
        "- **验收标准**：\n"
        "  1. 用户感觉系统表现良好。\n"
        "  2、用户觉得体验顺畅。\n"
        "  3) 界面令人满意。\n"
        "- **数据与界面契约**：\n"
        "  | 字段 | 类型 | 必填 | 约束 |\n"
        "  | --- | --- | --- | --- |\n"
        "  | 示例字段 | String | 是 | ≤ 64 字符 |\n"
        "- **来源**：§4.1.1\n" % req_id
    )


def _threshold_entry(req_id: str) -> str:
    """An entry decidable only through numeric thresholds, no backticks or state words."""
    return (
        "### %s 账户锁定阈值\n\n"
        "- **优先级**：必须\n"
        "- **用户故事**：作为项目管理员，我希望连续失败后锁定账户，以便抵御暴力破解。\n"
        "- **验收标准**：\n"
        "  1. 连续 5 次登录失败后锁定 30 分钟。\n"
        "  2. 第 31 分钟起计数归零，可再次尝试。\n"
        "- **数据与界面契约**：\n"
        "  | 字段 | 类型 | 必填 | 约束 |\n"
        "  | --- | --- | --- | --- |\n"
        "  | 锁定阈值 | Integer | 是 | 1–10 次 |\n"
        "- **来源**：§4.1.1\n" % req_id
    )


def _h2_sections(text: str) -> set:
    """Normalized names of every H2 heading in ``text``."""
    return set(section_requirement_counts(text))


def _area_table() -> str:
    rows = [
        "| %s | `docs/pid-platform/requirements/%s` | 示例范围 |"
        % (area, name)
        for area, name in REQ_AREA_FILES.items()
    ]
    return (
        "## 区域与文件映射\n\n"
        "| 区域 | 文档 | 范围 |\n"
        "| --- | --- | --- |\n" + "\n".join(rows) + "\n"
    )


def _matrix(rows: list) -> str:
    body = "\n".join(
        "| %s | %s | %s | %s |" % row for row in rows
    )
    return (
        "## 需求追溯矩阵\n\n"
        "| %s |\n"
        "| --- | --- | --- | --- |\n" % " | ".join(MATRIX_COLUMNS) + body + "\n"
    )


def _overview(matrix_rows_: list) -> str:
    return (
        "# 需求体系总纲\n\n"
        "## 需求 ID 规范\n\n"
        "需求 ID 的锚定正则为 `%s`。\n\n" % REQ_ID_TEXT
        + _area_table()
        + "\n"
        + _matrix(matrix_rows_)
        + "\n## 校验规则\n\n锚定、唯一、每区下限与文件对应逐条机械校验。\n"
    )


def _documents(platform_ids: list, type_ids: list, matrix: list) -> dict:
    platform = "# 平台层需求\n\n## 身份与会话\n\n" + "\n".join(
        _entry(req_id) for req_id in platform_ids
    )
    type_library = "# 类型库与属性模型需求\n\n## 业务类型树\n\n" + "\n".join(
        _entry(req_id) for req_id in type_ids
    )
    return {
        OVERVIEW_PATH: _overview(matrix),
        PLAT_PATH: platform,
        TYPE_PATH: type_library,
    }


def _valid_documents() -> dict:
    platform_ids = ["REQ-PLAT-%03d" % number for number in range(1, AREA_FLOOR + 1)]
    type_ids = ["REQ-TYPE-%03d" % number for number in range(1, AREA_FLOOR + 1)]
    matrix = [(req_id, "PLAT", "无界面", "平台认证服务承载，原型不含登录屏") for req_id in platform_ids]
    matrix += [(req_id, "TYPE", "typelib", "类型库编辑器屏承载类型树") for req_id in type_ids]
    return _documents(platform_ids, type_ids, matrix)


CONNECTION_POINT_ROWS = [
    ("等径", "双向点", "等径直通管件两端，介质可双向流动"),
    ("等径", "流入点", "等径单向流元件入口，介质只能流入"),
    ("等径", "流出点", "等径单向流元件出口，介质只能流出"),
    ("异径", "双向点", "异径管两端的变径连接，介质可双向流动"),
    ("异径", "流入点", "异径单向流元件入口，介质只能流入"),
    ("异径", "流出点", "异径单向流元件出口，介质只能流出"),
    ("信号线", "双向点", "仪表之间的双向信号传递"),
    ("信号线", "流入点", "进入仪表的单向信号"),
    ("信号线", "流出点", "离开仪表的单向信号"),
]


def _symbol_document() -> str:
    """A synthetic symbol-library document that satisfies every VAL-REQ-006 check.

    The negative controls below mutate one fact at a time, so each failure class
    is proven to be detected rather than merely described.
    """
    connection_table = (
        "| 类型 | 方向 | 用途 |\n"
        "| --- | --- | --- |\n"
        + "\n".join("| %s | %s | %s |" % row for row in CONNECTION_POINT_ROWS)
        + "\n"
    )
    tag_table = (
        "| 字段 | 类型 | 必填 | 约束 |\n"
        "| --- | --- | --- | --- |\n"
        + "\n".join("| %s | String | 是 | — |" % name for name in TAG_RULE_FIELDS)
        + "\n"
    )
    extras = {
        "连接点": connection_table,
        "位号规则": tag_table + "\n格式 `P-{序号}` 配起始编码 1001 生成 `P-1001`。\n\n",
        "颜色规则": "条件要素：属性、运算符、值、连接符；当前类型无规则时继承父类型规则。\n\n",
        "相交断线与打断符": (
            "7 种管线类型权重中夹套管最高、在役次工艺管线最低；"
            "打断符样式 3 种；打断符高度 默认 3。\n\n"
        ),
    }
    parts = ["# 符号库与项目设置需求\n\n共 %d 条需求。\n\n" % len(SYMBOL_SECTIONS)]
    for index, name in enumerate(SYMBOL_SECTIONS, start=1):
        parts.append("## %s\n\n" % name)
        parts.append(extras.get(name, ""))
        parts.append(_entry("REQ-SYM-%03d" % index, title="%s示例" % name))
        parts.append("\n")
    return "".join(parts)


def _drawing_document() -> str:
    """A synthetic 40-drawing document carrying every VAL-REQ-007 phrase."""
    extras = {
        "管件与阀门插入": "插入时管线自动断开，移动时自动补位。\n\n",
        "设备与管口": (
            "管口位号前缀继承设备位号；未设默认管口时生成逻辑连接点；"
            "关联设备属性共享。\n\n"
        ),
        "页间连接符": (
            "管道连接符流入、管道连接符流出、信号线连接符流入、"
            "信号线连接符流出。\n\n"
        ),
        "草图设置与用户环境": "草图设置随图纸保存，用户环境随用户保存。\n\n",
        "管线与管路层级": "管线 → 管路 → 管段，属性下传。\n\n",
        "仪表与仪表回路": (
            "回路与仪表多对多；支持新建仪表回路、添加仪表、移除仪表。\n\n"
        ),
        "图纸生命周期": "新建、导入 SVG/DWG、复制、移动、权限、预览。\n\n",
        "图纸版本与比对": "版本快照可两两比对。\n\n",
        "对象复制": "支持图纸内与跨图纸复制。\n\n",
    }
    parts = ["# 图纸与绘图需求\n\n共 %d 条需求。\n\n" % len(DRAWING_SECTIONS)]
    for index, name in enumerate(DRAWING_SECTIONS, start=1):
        parts.append("## %s\n\n" % name)
        parts.append(extras.get(name, ""))
        parts.append(_entry("REQ-DRW-%03d" % index, title="%s示例" % name))
        parts.append("\n")
    return "".join(parts)


# ---------------------------------------------------------------------------
# Real documents
# ---------------------------------------------------------------------------


def test_traceability_requirement_ids_follow_the_anchored_pattern():
    documents = load_all_documents(DOCS)
    assert documents, "no markdown documents under " + str(DOCS)
    errors = validate_requirement_ids(documents)
    assert errors == [], "requirement-ID errors: %s" % errors


def test_traceability_requirement_ids_are_globally_unique():
    documents = load_documents(DOCS)
    entries = [
        entry
        for text in documents.values()
        for entry in split_entries(text)
    ]
    assert entries, "no requirement headings found under " + str(DOCS)
    ids = [entry["id"] for entry in entries]
    duplicates = sorted({req_id for req_id in ids if ids.count(req_id) > 1})
    assert duplicates == [], "duplicate requirement IDs: %s" % duplicates
    assert len(definitions(documents)) == len(entries), (
        "definitions() collapsed %d entries into %d IDs"
        % (len(entries), len(definitions(documents)))
    )


def test_traceability_every_authored_area_document_meets_the_five_floor():
    documents = load_documents(DOCS)
    authored = [area for area, name in REQ_AREA_FILES.items() if "requirements/" + name in documents]
    assert "PLAT" in authored and "TYPE" in authored, (
        "the platform and type-library documents must be authored, found: %s" % authored
    )
    errors = validate_area_floors(documents)
    assert errors == [], "area floor errors: %s" % errors
    for area in authored:
        path = "requirements/" + REQ_AREA_FILES[area]
        count = len([entry for entry in split_entries(documents[path]) if entry["area"] == area])
        assert count >= AREA_FLOOR, "%s defines only %d %s requirements" % (path, count, area)


def test_traceability_area_segment_matches_the_owning_document():
    documents = load_documents(DOCS)
    errors = validate_requirement_ids(documents)
    misplaced = [error for error in errors if "belongs in" in error or "unknown area" in error]
    assert misplaced == [], "area/file mismatches: %s" % misplaced


def test_traceability_overview_documents_the_anchored_regex():
    documents = load_documents(DOCS)
    overview = documents.get(OVERVIEW_PATH)
    assert overview is not None, "%s is missing" % OVERVIEW_PATH
    assert REQ_ID_TEXT in overview, (
        "%s does not carry the anchored pattern verbatim: %s" % (OVERVIEW_PATH, REQ_ID_TEXT)
    )
    assert "(?<![A-Za-z-])" in overview and "(?!\\d)" in overview, (
        "the documented pattern is missing a negative lookaround"
    )
    assert REQ_ID_RE.pattern == REQ_ID_TEXT
    assert conftest.REQ_ID_RE.pattern == REQ_ID_TEXT, (
        "conftest and requirements_contract disagree on the anchored pattern"
    )


def test_traceability_area_table_maps_nine_areas_to_nine_files():
    documents = load_documents(DOCS)
    overview = documents.get(OVERVIEW_PATH)
    assert overview is not None, "%s is missing" % OVERVIEW_PATH
    errors = validate_area_table(overview)
    assert errors == [], "area table errors: %s" % errors
    assert conftest.REQ_AREA_FILES == REQ_AREA_FILES, (
        "conftest.REQ_AREA_FILES diverged from requirements_contract.REQ_AREA_FILES"
    )


def test_traceability_matrix_and_definitions_are_mutually_complete():
    documents = load_documents(DOCS)
    overview = documents[OVERVIEW_PATH]
    defined = definitions(documents)
    errors = validate_matrix(overview, defined, _screen_ids())
    missing = [error for error in errors if "missing from the traceability matrix" in error]
    orphans = [error for error in errors if "has no requirement definition" in error]
    assert missing == [], "requirements omitted from the matrix: %s" % missing
    assert orphans == [], "matrix rows without a definition: %s" % orphans
    assert errors == [], "matrix errors: %s" % errors
    assert {row["id"] for row in matrix_rows(overview)} == set(defined)


def test_traceability_matrix_screen_values_are_catalog_screens_or_interface_free():
    documents = load_documents(DOCS)
    rows = matrix_rows(documents[OVERVIEW_PATH])
    assert rows, "the traceability matrix has no rows"
    allowed = set(_screen_ids()) | {"无界面"}
    unknown = [
        "%s: %s" % (row["line"], row["screen"])
        for row in rows
        if row["screen"] not in allowed
    ]
    assert unknown == [], "matrix rows use unknown screens: %s" % unknown
    interface_free = [row for row in rows if row["screen"] == "无界面"]
    assert interface_free, "expected at least one 无界面 row (login, audit, config)"
    thin = [row["id"] for row in interface_free if len(row["note"]) < 8]
    assert thin == [], "无界面 rows without a reason: %s" % thin


def test_traceability_every_requirement_entry_has_the_five_fields():
    documents = load_documents(DOCS)
    errors = validate_entries(documents, allowed=whitelisted_sections())
    assert errors == [], "requirement entry errors: %s" % errors


def test_traceability_platform_document_covers_the_six_topics():
    errors = validate_platform_coverage(load_documents(DOCS))
    assert errors == [], "platform coverage errors: %s" % errors


def test_traceability_type_library_document_covers_the_seven_topics():
    errors = validate_type_library_coverage(load_documents(DOCS))
    assert errors == [], "type-library coverage errors: %s" % errors


def test_traceability_symbol_document_covers_the_thirteen_topics():
    errors = validate_symbol_library_coverage(load_documents(DOCS))
    assert errors == [], "symbol-library coverage errors: %s" % errors


def test_traceability_drawing_document_covers_the_thirteen_topics():
    errors = validate_drawing_coverage(load_documents(DOCS))
    assert errors == [], "drawing coverage errors: %s" % errors


def test_traceability_drawing_connectors_split_kind_by_direction():
    """VAL-REQ-007: 管道/信号线 x 流入/流出, four symbol types, none merged."""
    documents = load_documents(DOCS)
    connectors = section_text(documents[DRAWING_PATH], "页间连接符")
    for kind in ("管道连接符", "信号线连接符"):
        for direction in ("流入", "流出"):
            token = kind + direction
            assert token in connectors, "页间连接符 does not name '%s'" % token


def test_traceability_symbol_document_lists_nine_connection_point_combinations():
    """VAL-REQ-006: 3 types x 3 directions, each row carrying a 用途."""
    documents = load_documents(DOCS)
    connection = section_text(documents[SYMBOL_PATH], "连接点")
    assert validate_connection_point_table(connection) == [], "连接点 table errors: %s" % (
        validate_connection_point_table(connection),
    )
    rows = [
        line
        for line in connection.splitlines()
        if line.startswith("|") and "---" not in line
    ]
    # header + 9 data rows: the combination table is the only 3-column table here
    data_rows = [line for line in rows if line.count("|") == 4]
    assert len(data_rows) == 10, "expected the header and 9 rows, got %d: %s" % (
        len(data_rows),
        data_rows,
    )
    for kind in CONNECTION_POINT_TYPES:
        for direction in CONNECTION_POINT_DIRECTIONS:
            assert any(
                ("| %s | %s |" % (kind, direction)) in line for line in data_rows
            ), "%s/%s is missing from the 连接点 table" % (kind, direction)


def test_traceability_symbol_document_defines_the_eight_tag_rule_fields():
    documents = load_documents(DOCS)
    tag_rules = section_text(documents[SYMBOL_PATH], "位号规则")
    assert len(TAG_RULE_FIELDS) == 8, TAG_RULE_FIELDS
    for name in TAG_RULE_FIELDS:
        assert name in tag_rules, "位号规则 does not define '%s'" % name
    assert "P-1001" in tag_rules, "位号规则 carries no concrete 格式拼接 example"


def test_traceability_screen_catalog_uses_the_fourteen_contract_ids():
    assert len(SCREEN_IDS) == 15, "VAL-CROSS-001 lists 15 screen ids, got %d" % len(SCREEN_IDS)
    if not CATALOG.exists():
        assert _screen_ids() == SCREEN_IDS
        return
    assert _screen_ids() == SCREEN_IDS, (
        "screen-catalog.json ids differ from the contract list: %s" % _screen_ids()
    )


# ---------------------------------------------------------------------------
# Positive control: a synthetic but valid set must pass every validator
# ---------------------------------------------------------------------------


def test_traceability_synthetic_valid_documents_pass_every_validator():
    documents = _valid_documents()
    assert validate_requirement_ids(documents) == []
    assert validate_area_floors(documents, require_all=False) == []
    assert validate_area_table(documents[OVERVIEW_PATH]) == []
    assert validate_matrix(documents[OVERVIEW_PATH], definitions(documents), SCREEN_IDS) == []
    assert validate_entries(documents, allowed=whitelisted_sections()) == []


# ---------------------------------------------------------------------------
# Negative controls: one per failure class, each must name its location
# ---------------------------------------------------------------------------


def test_traceability_final_mode_requires_all_nine_area_documents():
    """The closeout mode must name every area document that is still absent."""
    errors = validate_area_floors(_valid_documents(), require_all=True)
    missing = [error for error in errors if "is missing" in error]
    assert len(missing) == 7, (
        "expected the seven unauthored areas to be reported, got: %s" % errors
    )
    for area in ["SYM", "DRW", "DATA", "DELV", "COLL", "IO", "MIG"]:
        assert any(area in error for error in missing), (area, missing)


def test_traceability_validator_rejects_a_definition_outside_requirements():
    documents = _valid_documents()
    documents["PRD.md"] = "# 主 PRD\n\n### REQ-PLAT-950 主文档里定义需求\n\n" + _entry(
        "REQ-PLAT-950", title="主文档里定义需求"
    )
    errors = validate_requirement_ids(documents)
    assert any("outside the requirements directory" in error for error in errors), errors


def test_traceability_validator_rejects_a_duplicate_requirement_id():
    documents = _valid_documents()
    documents[PLAT_PATH] += "\n" + _entry("REQ-PLAT-001", title="重复定义")
    errors = validate_requirement_ids(documents)
    duplicates = [error for error in errors if "is defined 2 times" in error]
    assert duplicates, "the duplicate-ID control passed: %s" % errors
    assert "REQ-PLAT-001" in duplicates[0] and PLAT_PATH in duplicates[0], duplicates[0]


def test_traceability_validator_rejects_a_wrong_area_segment():
    documents = _valid_documents()
    documents[PLAT_PATH] += "\n" + _entry("REQ-TYPE-901", title="错区域")
    errors = validate_requirement_ids(documents)
    misplaced = [error for error in errors if "belongs in" in error]
    assert misplaced, "the wrong-area control passed: %s" % errors
    assert "REQ-TYPE-901" in misplaced[0] and "20-type-library.md" in misplaced[0], misplaced[0]


def test_traceability_validator_rejects_an_id_missing_from_the_matrix():
    documents = _valid_documents()
    overview = documents[OVERVIEW_PATH]
    broken = overview.replace(
        "| REQ-PLAT-005 | PLAT | 无界面 | 平台认证服务承载，原型不含登录屏 |\n", ""
    )
    assert broken != overview, "the control could not remove the matrix row"
    documents[OVERVIEW_PATH] = broken
    errors = validate_matrix(broken, definitions(documents), SCREEN_IDS)
    missing = [error for error in errors if "missing from the traceability matrix" in error]
    assert missing, "the omission control passed: %s" % errors
    assert "REQ-PLAT-005" in missing[0] and PLAT_PATH in missing[0], missing[0]


def test_traceability_validator_rejects_an_orphan_matrix_row():
    documents = _valid_documents()
    overview = documents[OVERVIEW_PATH]
    broken = overview.replace(
        "| REQ-TYPE-005 | TYPE | typelib | 类型库编辑器屏承载类型树 |",
        "| REQ-TYPE-005 | TYPE | typelib | 类型库编辑器屏承载类型树 |\n"
        "| REQ-TYPE-777 | TYPE | typelib | 没有定义的行 |",
    )
    assert broken != overview, "the control could not append the orphan row"
    errors = validate_matrix(broken, definitions(documents), SCREEN_IDS)
    orphans = [error for error in errors if "has no requirement definition" in error]
    assert orphans, "the orphan control passed: %s" % errors
    assert "REQ-TYPE-777" in orphans[0] and OVERVIEW_PATH in orphans[0], orphans[0]


def test_traceability_validator_rejects_an_unknown_screen_id():
    documents = _valid_documents()
    overview = documents[OVERVIEW_PATH]
    broken = overview.replace(
        "| REQ-TYPE-005 | TYPE | typelib |", "| REQ-TYPE-005 | TYPE | dashboard |"
    )
    assert broken != overview, "the control could not replace the screen value"
    errors = validate_matrix(broken, definitions(documents), SCREEN_IDS)
    unknown = [error for error in errors if "unknown screen 'dashboard'" in error]
    assert unknown, "the unknown-screen control passed: %s" % errors
    assert "REQ-TYPE-005" in unknown[0] and OVERVIEW_PATH in unknown[0], unknown[0]


def test_traceability_validator_rejects_interface_free_without_a_reason():
    documents = _valid_documents()
    overview = documents[OVERVIEW_PATH]
    broken = overview.replace(
        "| REQ-PLAT-005 | PLAT | 无界面 | 平台认证服务承载，原型不含登录屏 |",
        "| REQ-PLAT-005 | PLAT | 无界面 | — |",
    )
    assert broken != overview, "the control could not strip the reason"
    errors = validate_matrix(broken, definitions(documents), SCREEN_IDS)
    thin = [error for error in errors if "does not state a reason" in error]
    assert thin, "the 无界面-reason control passed: %s" % errors
    assert "REQ-PLAT-005" in thin[0] and OVERVIEW_PATH in thin[0], thin[0]


def test_traceability_validator_rejects_a_malformed_id_digit_count():
    documents = _valid_documents()
    documents[TYPE_PATH] += "\n" + _entry("REQ-TYPE-12", title="两位编号")
    errors = validate_requirement_ids(documents)
    malformed = [error for error in errors if "does not match the anchored pattern" in error]
    assert malformed, "the malformed-ID control passed: %s" % errors
    assert "REQ-TYPE-12" in malformed[0], malformed[0]


def test_traceability_validator_rejects_a_definition_in_the_overview():
    documents = _valid_documents()
    documents[OVERVIEW_PATH] += "\n" + _entry("REQ-PLAT-900", title="总纲里定义需求")
    errors = validate_requirement_ids(documents)
    reference_only = [error for error in errors if "must not define" in error]
    assert reference_only, "the reference-only control passed: %s" % errors
    assert OVERVIEW_PATH in reference_only[0], reference_only[0]


def test_traceability_validator_rejects_a_thin_area_document():
    documents = _valid_documents()
    documents[TYPE_PATH] = (
        "# 类型库与属性模型需求\n\n## 业务类型树\n\n" + _entry("REQ-TYPE-001")
    )
    errors = validate_area_floors(documents)
    assert any("area TYPE defines 1" in error for error in errors), errors
    assert any(TYPE_PATH in error for error in errors), errors


def test_traceability_validator_rejects_a_missing_requirement_field():
    documents = _valid_documents()
    broken = _entry("REQ-PLAT-001").replace(
        "- **数据与界面契约**：\n"
        "  | 字段 | 类型 | 必填 | 约束 |\n"
        "  | --- | --- | --- | --- |\n"
        "  | 示例字段 | String | 是 | ≤ 64 字符 |\n",
        "",
    )
    documents[PLAT_PATH] = "# 平台层需求\n\n## 身份与会话\n\n" + broken
    errors = validate_entries(documents)
    assert any("数据与界面契约" in error for error in errors), errors


def test_traceability_validator_rejects_an_unverified_source_section():
    documents = _valid_documents()
    documents[PLAT_PATH] += "\n" + _entry("REQ-PLAT-902", source="§9.9.9.9")
    errors = validate_entries(documents, allowed=whitelisted_sections())
    assert any("9.9.9.9" in error for error in errors), errors


def test_traceability_validator_rejects_vague_acceptance_criteria():
    """VAL-REQ-002 decidability can fail: a criterion's own list number is not evidence.

    The three criteria below use the three list markers the contract accepts
    (``1.`` / ``2、`` / ``3)``). Before the marker was stripped, its digit
    satisfied the "carries a number" branch and every one of them passed.
    """
    documents = _valid_documents()
    documents[PLAT_PATH] = (
        "# 平台层需求\n\n## 身份与会话\n\n" + _vague_entry("REQ-PLAT-001")
    )
    errors = validate_entries(documents)
    vague = [
        error
        for error in errors
        if "no number, named command or observable state" in error
    ]
    assert len(vague) == 3, (
        "expected all three sentiment-only criteria to be rejected, got: %s" % errors
    )
    for index in (1, 2, 3):
        matching = [error for error in vague if "criterion %d" % index in error]
        assert matching, "criterion %d was not rejected: %s" % (index, vague)
        assert "REQ-PLAT-001" in matching[0] and PLAT_PATH in matching[0], matching[0]


def test_traceability_validator_keeps_numeric_threshold_criteria_decidable():
    """Positive control: a threshold-only criterion still passes.

    Both criteria are checked to carry neither a backticked token nor an
    observable word, so only the numeric branch can accept them.
    """
    for criterion in [
        "1. 连续 5 次登录失败后锁定 30 分钟。",
        "2. 第 31 分钟起计数归零，可再次尝试。",
    ]:
        body = strip_list_number(criterion)
        assert "`" not in body, criterion
        assert not any(word in body for word in OBSERVABLE_WORDS), (
            "%s would pass on an observable word rather than its numbers" % criterion
        )
    documents = _valid_documents()
    documents[PLAT_PATH] = (
        "# 平台层需求\n\n## 身份与会话\n\n" + _threshold_entry("REQ-PLAT-001")
    )
    errors = validate_entries(documents, allowed=whitelisted_sections())
    assert errors == [], "a numeric threshold must stay decidable: %s" % errors


@pytest.mark.parametrize("path,section", SECTION_HEADINGS)
def test_traceability_section_names_appear_as_h2_in_their_area_document(path, section):
    """Each declared section must be an H2 heading of the document that owns it."""
    text = load_documents(DOCS)[path]
    present = _h2_sections(text)
    assert normalize_heading(section) in present, (
        "%s does not carry '%s' as an H2 heading; found %s"
        % (path, section, sorted(present))
    )


def test_traceability_section_heading_check_rejects_a_renamed_heading():
    """Negative control: the H2 extraction really notices a heading going away."""
    documents = load_documents(DOCS)
    renamed = documents[PLAT_PATH].replace("## 身份与会话", "## 身份与安全")
    assert renamed != documents[PLAT_PATH], "the control could not rename the heading"
    assert normalize_heading("身份与会话") in _h2_sections(documents[PLAT_PATH])
    assert normalize_heading("身份与会话") not in _h2_sections(renamed)


def test_traceability_symbol_validator_accepts_the_synthetic_document():
    """Positive control: the synthetic document satisfies every VAL-REQ-006 check."""
    errors = validate_symbol_library_coverage({SYMBOL_PATH: _symbol_document()})
    assert errors == [], "the synthetic symbol document must pass: %s" % errors


def test_traceability_symbol_validator_rejects_a_missing_connection_combination():
    """Negative control: dropping one of the nine rows must name the missing pair."""
    document = _symbol_document().replace(
        "| 信号线 | 流出点 | 离开仪表的单向信号 |\n", ""
    )
    assert "| 信号线 | 流出点 |" not in document, "the control removed nothing"
    errors = validate_symbol_library_coverage({SYMBOL_PATH: document})
    assert any("信号线/流出点" in error for error in errors), errors
    assert any("8 rows" in error for error in errors), errors


def test_traceability_symbol_validator_rejects_a_repeated_connection_combination():
    """Negative control: a duplicate pair plus a hole must both be reported."""
    document = _symbol_document().replace(
        "| 异径 | 流入点 | 异径单向流元件入口，介质只能流入 |",
        "| 等径 | 双向点 | 重复的一行 |",
    )
    errors = validate_symbol_library_coverage({SYMBOL_PATH: document})
    assert any("等径/双向点 repeats" in error for error in errors), errors
    assert any("omits the 异径/流入点 combination" in error for error in errors), errors


def test_traceability_symbol_validator_rejects_a_missing_tag_rule_field():
    """Negative control: an absent field name must be reported by name."""
    document = _symbol_document().replace("| 后缀递增 | String | 是 | — |\n", "")
    assert "后缀递增" not in document, "the control removed nothing"
    errors = validate_symbol_library_coverage({SYMBOL_PATH: document})
    assert any("does not define the field '后缀递增'" in error for error in errors), errors


def test_traceability_symbol_validator_rejects_a_tag_rule_field_synonym():
    """Negative control: VAL-DOC-009 forbids rewording the field inside its own document."""
    document = _symbol_document().replace("位号唯一校验", "位号唯一性校验")
    errors = validate_symbol_library_coverage({SYMBOL_PATH: document})
    assert any("uses the synonym" in error for error in errors), errors


def test_traceability_symbol_validator_rejects_a_renamed_section():
    """Negative control: renaming one of the thirteen sections must be detected."""
    document = _symbol_document().replace("## 绝热等级\n", "## 绝热等级设置\n")
    errors = validate_symbol_library_coverage({SYMBOL_PATH: document})
    assert any("missing section '绝热等级'" in error for error in errors), errors


def test_traceability_symbol_validator_rejects_a_missing_color_rule_element():
    """Negative control: the four condition elements are each load-bearing."""
    document = _symbol_document().replace(
        "条件要素：属性、运算符、值、连接符；当前类型无规则时继承父类型规则。",
        "条件要素：属性、运算符、值；当前类型无规则时继承父类型规则。",
    )
    errors = validate_symbol_library_coverage({SYMBOL_PATH: document})
    assert any("condition element '连接符'" in error for error in errors), errors


def test_traceability_symbol_validator_rejects_a_missing_document():
    """Negative control: an absent area document is reported, not silently skipped."""
    errors = validate_symbol_library_coverage({})
    assert errors == ["%s: symbol library requirement document is missing" % SYMBOL_PATH]


# ---------------------------------------------------------------------------
# Drawing document (VAL-REQ-007) controls
# ---------------------------------------------------------------------------


def test_traceability_drawing_validator_accepts_the_synthetic_document():
    """Positive control: the synthetic document satisfies every VAL-REQ-007 check."""
    errors = validate_drawing_coverage({DRAWING_PATH: _drawing_document()})
    assert errors == [], "the synthetic drawing document must pass: %s" % errors


def test_traceability_drawing_validator_rejects_a_missing_break_and_fill_semantics():
    """Negative control: VAL-REQ-007's auto-break/auto-fill pair is load-bearing."""
    document = _drawing_document().replace(
        "插入时管线自动断开，移动时自动补位。",
        "插入时可选择断开管线。",
    )
    assert "自动补位" not in document, "the control removed nothing"
    errors = validate_drawing_coverage({DRAWING_PATH: document})
    assert any(
        "管件与阀门插入 does not state '自动补位'" in error for error in errors
    ), errors


def test_traceability_drawing_validator_rejects_a_missing_connector_kind():
    """Negative control: dropping 信号线连接符流出 must name that kind."""
    document = _drawing_document().replace("信号线连接符流出。", "。")
    assert "信号线连接符流出" not in document, "the control removed nothing"
    errors = validate_drawing_coverage({DRAWING_PATH: document})
    assert any(
        "页间连接符 does not state '信号线连接符流出'" in error for error in errors
    ), errors


def test_traceability_drawing_validator_rejects_a_renamed_section():
    """Negative control: renaming one of the thirteen sections must be detected."""
    document = _drawing_document().replace("## 标注\n", "## 对象标注\n")
    errors = validate_drawing_coverage({DRAWING_PATH: document})
    assert any("missing section '标注'" in error for error in errors), errors


def test_traceability_drawing_validator_rejects_a_section_without_requirements():
    """Negative control: a present-but-empty section is not coverage."""
    parts = _drawing_document().split("\n## ")
    trimmed = []
    removed = False
    for part in parts:
        if part.startswith("打断符"):
            head, separator, _rest = part.partition("### REQ-DRW-")
            assert separator, "打断符 carries no requirement entry to remove"
            trimmed.append(head.rstrip() + "\n\n")
            removed = True
        else:
            trimmed.append(part)
    document = "\n## ".join(trimmed)
    assert removed, "the control did not touch the 打断符 section"
    empty_section = document.split("\n## 打断符")[1].split("\n## ")[0]
    assert "### REQ-DRW-" not in empty_section, "the entry survived the removal"
    errors = validate_drawing_coverage({DRAWING_PATH: document})
    assert any(
        "section '打断符' has 0 requirements" in error for error in errors
    ), errors


def test_traceability_drawing_validator_rejects_a_missing_document():
    """Negative control: an absent area document is reported, not silently skipped."""
    errors = validate_drawing_coverage({})
    assert errors == ["%s: drawing requirement document is missing" % DRAWING_PATH]


# ---------------------------------------------------------------------------
# Data & rule engine document (VAL-REQ-008)
# ---------------------------------------------------------------------------

RESULT_TABLE = (
    "| 字段 | 类型 | 必填 | 约束 |\n"
    "| --- | --- | --- | --- |\n"
    "| 规则 ID | String | 是 | 形如 DR-001 |\n"
    "| 对象引用 | ObjectRef | 是 | 类型编码、位号与图纸 ID |\n"
    "| 严重度 | Enum(错误/警告/提示) | 是 | 三值枚举 |\n"
    "| 消息 | String | 是 | 不超过 200 字符 |\n"
    "| 建议修复 | String | 否 | 不超过 200 字符 |\n"
)

TRIGGER_TABLE = (
    "| 触发时机 | 执行范围 | 阻塞行为 | 结果处理 |\n"
    "| --- | --- | --- | --- |\n"
    "| 按需 | 当前图纸 | 不阻断 | 写入检查结果集 |\n"
    "| 保存前 | 当前图纸 | 阻断 | 返回 `RULE_BLOCKED` |\n"
    "| 发布前 | 待发布图纸 | 阻断 | 返回 `PUBLISH_BLOCKED` |\n"
)


def _rule_table(rows) -> str:
    body = "\n".join("| `%s` | %s | %s | %s | `%s` |" % row for row in rows)
    return (
        "| %s |\n| --- | --- | --- | --- | --- |\n" % " | ".join(RULE_TABLE_COLUMNS)
        + body
        + "\n"
    )


def _data_document() -> str:
    """A synthetic 50 document that satisfies every VAL-REQ-008 check."""
    extras = {
        "位号主数据与唯一性": (
            "位号主数据以 `(类型编码, 位号)` 为唯一键，`位号` 与 `类型编码` 组合后唯一。\n\n"
            "唯一性在项目级生效，唯一键为 `(类型编码, 位号)` 复合键：同一类型下的两个实例"
            "不得登记相同位号；不同类型对象允许同值位号。\n\n"
        ),
        "规则 DSL": "```ebnf\n" + rule_dsl.EBNF_GRAMMAR + "\n```\n\n",
        "位号冲突处理": (
            "位号冲突在登记新实例时检出，并在校验结果中记入 1 条 `DR-004` 错误。\n\n"
            "`DR-004` 由设计规则检查独立执行，不随 `位号唯一校验` 关闭而停用："
            "它作为规则表条目在 `按需`、`保存前`、`发布前` 三种触发时机求值。\n\n"
        ),
        "触发时机": TRIGGER_TABLE + "\n",
        "结果模型": RESULT_TABLE + "\n",
        "设计规则清单": _rule_table(SHIPPED_DESIGN_RULES) + "\n",
    }
    parts = ["# 数据管理与规则检查需求\n\n共 %d 条需求。\n\n" % len(DATA_SECTIONS)]
    for index, name in enumerate(DATA_SECTIONS, start=1):
        parts.append("## %s\n\n" % name)
        parts.append(extras.get(name, ""))
        parts.append(_entry("REQ-DATA-%03d" % index, title="%s示例" % name))
        parts.append("\n")
    return "".join(parts)


def test_traceability_data_validator_accepts_the_synthetic_document():
    """Positive control: the synthetic document satisfies every VAL-REQ-008 check."""
    errors = validate_data_rule_engine_coverage({DATA_PATH: _data_document()})
    assert errors == [], "the synthetic data document must pass: %s" % errors


def test_traceability_data_document_satisfies_the_contract():
    """Real document: nine sections, the EBNF block, the rule table, the result model."""
    errors = validate_data_rule_engine_coverage(load_documents(DOCS))
    assert errors == [], "VAL-REQ-008 errors: %s" % errors


def test_traceability_data_document_meets_the_element_and_rule_floors():
    """Real document: at least nine REQ-DATA entries and at least twenty rules."""
    text = load_documents(DOCS)[DATA_PATH]
    ids = {entry["id"] for entry in split_entries(text)}
    assert len(ids) >= 9, "the data document defines %d requirements" % len(ids)
    table = find_table(section_text(text, "设计规则清单"), RULE_TABLE_COLUMNS)
    assert table is not None, "no design-rule table found"
    assert len(table["rows"]) >= 20, "%d rules shipped" % len(table["rows"])
    parsed = [
        (
            _plain_cell(row[0]),
            _plain_cell(row[1]),
            _plain_cell(row[2]),
            _plain_cell(row[3]),
            _plain_cell(row[4]),
        )
        for row in table["rows"]
    ]
    assert parsed[: len(SHIPPED_DESIGN_RULES)] == [
        tuple(row) for row in SHIPPED_DESIGN_RULES
    ], "the shipped rules drifted from the design contract"


def test_traceability_data_validator_rejects_a_renamed_section():
    """Negative control: renaming one of the nine sections must be detected."""
    document = _data_document().replace("## 结果模型\n", "## 检查结果模型\n")
    errors = validate_data_rule_engine_coverage({DATA_PATH: document})
    assert any("missing section '结果模型'" in error for error in errors), errors


def test_traceability_data_validator_rejects_a_missing_ebnf_block():
    """Negative control: prose about the grammar is not a grammar."""
    document = _data_document().replace("```ebnf", "```text")
    errors = validate_data_rule_engine_coverage({DATA_PATH: document})
    assert any("no fenced code block tagged ebnf" in error for error in errors), errors


def test_traceability_data_validator_rejects_an_incomplete_grammar():
    """Negative control: dropping the operator production must name it."""
    document = _data_document().replace(
        'operator      = "=" | "!="',
        'op            = "=" | "!="',
    )
    assert "operator      =" not in document, "the control renamed nothing"
    errors = validate_rule_dsl_grammar(document)
    assert any("production 'operator'" in error for error in errors), errors


def test_traceability_data_validator_rejects_a_missing_character_production():
    """Negative control: a referenced but undefined nonterminal must be named."""
    grammar = rule_dsl.EBNF_GRAMMAR
    stripped = "\n".join(
        line
        for line in grammar.splitlines()
        if not line.strip().startswith("character ")
    )
    assert stripped != grammar, "the control removed nothing"
    assert "character     =" not in stripped, "the production survived the strip"
    # ``string`` still mentions the nonterminal, so the reference really dangles.
    assert "character" in stripped, "the control also deleted the reference"
    assert undefined_ebnf_nonterminals(grammar) == [], "the shipped grammar is broken"
    assert undefined_ebnf_nonterminals(stripped) == ["character"]
    document = _data_document().replace(grammar, stripped)
    assert stripped in document, "the control did not reach the document"
    errors = validate_rule_dsl_grammar(document)
    assert any(
        "'character'" in error and "no production defines it" in error
        for error in errors
    ), errors


def test_traceability_data_validator_accepts_the_shipped_grammar():
    """Positive control: every nonterminal in the shipped block is defined."""
    assert undefined_ebnf_nonterminals(rule_dsl.EBNF_GRAMMAR) == []
    assert "character" in ebnf_productions(rule_dsl.EBNF_GRAMMAR)


def test_traceability_data_validator_rejects_fewer_than_twenty_rules():
    """Negative control: a short rule list is counted, not estimated."""
    document = _data_document().replace(
        _rule_table(SHIPPED_DESIGN_RULES),
        _rule_table(SHIPPED_DESIGN_RULES[:14]),
    )
    assert "DR-025" not in document, "the control kept the full table"
    errors = validate_design_rule_table(document)
    assert any("14 rules; at least 20 required" in error for error in errors), errors


def test_traceability_data_validator_rejects_an_unparseable_rule_expression():
    """Negative control: '==' is not in the grammar, so the row must be rejected."""
    document = _data_document().replace("`管道等级 = NULL`", "`管道等级 == NULL`")
    assert "`管道等级 == NULL`" in document, "the control changed nothing"
    errors = validate_design_rule_table(document)
    assert any("does not parse" in error and "DR-001" in error for error in errors), errors


def test_traceability_data_validator_rejects_an_unknown_severity():
    """Negative control: the severity vocabulary is closed at three values."""
    document = _data_document().replace(
        "| `DR-004` | 位号重复 | 对象实例 | 错误 |",
        "| `DR-004` | 位号重复 | 对象实例 | 严重 |",
    )
    assert "| 严重 |" in document, "the control changed nothing"
    errors = validate_design_rule_table(document)
    assert any("has severity '严重'" in error for error in errors), errors


def test_traceability_data_validator_rejects_a_missing_rule():
    """Negative control: a dropped mandated rule is named by ID."""
    document = _data_document().replace(
        "| `DR-009` | 图纸未设图例标准 | 图纸 | 错误 | `图例标准 = NULL` |\n", ""
    )
    assert "DR-009" not in document, "the control removed nothing"
    errors = validate_design_rule_table(document)
    assert any("omits the required rule DR-009" in error for error in errors), errors


def test_traceability_data_validator_rejects_a_renamed_severity_enum():
    """Negative control: 错误/警告/提示 is exact; dropping one value must fail."""
    document = _data_document().replace("Enum(错误/警告/提示)", "Enum(错误/警告)")
    errors = validate_result_model(document)
    assert any("must be exactly 错误/警告/提示" in error for error in errors), errors


def test_traceability_data_validator_rejects_a_missing_result_field():
    """Negative control: each of the five result fields is load-bearing."""
    document = _data_document().replace(
        "| 建议修复 | String | 否 | 不超过 200 字符 |\n", ""
    )
    assert "建议修复" not in document, "the control removed nothing"
    errors = validate_result_model(document)
    assert any("does not define the field '建议修复'" in error for error in errors), errors


def test_traceability_data_validator_rejects_a_missing_trigger_mode():
    """Negative control: all three trigger modes must be distinguished.

    The assertion is on the table row, not on the bare word: the synthetic
    位号冲突处理 prose legitimately mentions 发布前 as one of the three trigger
    timings, so a whole-document ``not in`` check would be a false control.
    """
    document = _data_document().replace(
        "| 发布前 | 待发布图纸 | 阻断 | 返回 `PUBLISH_BLOCKED` |\n", ""
    )
    assert "| 发布前 |" not in document, "the control removed nothing"
    errors = validate_trigger_modes(document)
    assert any("'发布前' mode" in error for error in errors), errors


def test_traceability_data_validator_rejects_a_trigger_table_without_blocking():
    """Negative control: naming the modes without a blocking behaviour is not enough."""
    document = _data_document().replace("不阻断", "放行").replace("| 阻断 |", "| 放行 |")
    assert "阻断" not in document, "the control left a blocking statement"
    errors = validate_trigger_modes(document)
    assert any("states no blocking behaviour" in error for error in errors), errors


def test_traceability_data_validator_rejects_a_missing_document():
    """Negative control: an absent area document is reported, not silently skipped."""
    errors = validate_data_rule_engine_coverage({})
    assert errors == [
        "%s: data and rule engine requirement document is missing" % DATA_PATH
    ]


def test_traceability_data_document_states_the_dr004_basis():
    """Real document: 50 must state that DR-004 outlives the uniqueness switch.

    60-deliverables-and-publish.md:11 promises the off-state behaviour; this is
    the basis for that promise, and it lives in the document that owns the rule
    engine. Reading 50:19/:44/:57 literally without it means switching the
    check off stops detection at registration time and the promise dangles.
    """
    text = load_documents(DOCS)[DATA_PATH]
    errors = validate_dr004_independence(text)
    assert errors == [], errors
    section = section_text(text, "位号冲突处理")
    assert "由设计规则检查独立执行" in section, (
        "the independence sentence is not inside 位号冲突处理"
    )


def test_traceability_dr004_independence_guard_rejects_a_collapsed_paragraph():
    """Negative control: collapsing the sentence must turn the guard red."""
    text = load_documents(DOCS)[DATA_PATH]
    section = section_text(text, "位号冲突处理")
    anchor = "由设计规则检查独立执行"
    paragraph = next(
        block.strip()
        for block in section.split("\n\n")
        if anchor in block
    )
    assert paragraph, "the control found no paragraph carrying the anchor"
    degraded = text.replace(paragraph, "`DR-004` 是内置规则。")
    assert degraded != text, "the control changed nothing"
    errors = validate_dr004_independence(degraded)
    assert errors, "a collapsed independence paragraph was accepted"


def test_traceability_dr004_independence_guard_rejects_a_dropped_trigger_mode():
    """Negative control: dropping 发布前 must name that token, not just fail."""
    text = load_documents(DOCS)[DATA_PATH]
    degraded = text.replace(
        "`按需`、`保存前`、`发布前` 三种触发时机求值",
        "`按需`、`保存前` 两种触发时机求值",
    )
    assert degraded != text, "the control changed nothing"
    errors = validate_dr004_independence(degraded)
    assert any("发布前" in error for error in errors), errors


# ---------------------------------------------------------------------------
# Deliverables document (VAL-REQ-009)
# ---------------------------------------------------------------------------

BOM_TABLE = (
    "| 序号 | 类型 | 位号 | 名称 | 数量 | 单位 |\n"
    "| --- | --- | --- | --- | --- | --- |\n"
    "| 1 | 管线 | L-1001 | 工艺水进料管线 | 24.5 | m |\n"
    "| 2 | 设备 | P-1001 | 离心泵 | 1 | 台 |\n"
    "| 3 | 阀门 | V-1001 | 截止阀 DN50 | 1 | 个 |\n"
    "| 4 | 仪表 | TI-1001 | 温度变送器 | 1 | 台 |\n"
    "| 5 | 管件 | L-1001-01 | 90°弯头 DN50 | 3 | 个 |\n"
)

EXPORT_TABLE = (
    "| 格式 | 导出对象 | 支持程度 | 限制 |\n"
    "| --- | --- | --- | --- |\n"
    "| DWG | 图纸 | 完整支持 | 不写位号主数据 |\n"
    "| DXF | 图纸 | 完整支持 | 不保留语义扩展数据 |\n"
    "| PDF | 图纸与报表 | 完整支持 | 只读 |\n"
    "| SVG | 图纸 | 完整支持 | 仅当前图纸 |\n"
    "| XLSX | 材料清单与报表 | 完整支持 | 列映射由模板声明 |\n"
    "| CSV | 材料清单与校验结果 | 完整支持 | 不含样式 |\n"
    "| JSON | 对象属性与发布清单 | 完整支持 | 遵循统一导出规则 |\n"
)

BOM_BODY = (
    "材料清单的行键是位号主数据使用的复合键 `(类型编码, 位号)`：同一 `类型编码` 下每个位号恰占一行，"
    "一个位号在同一份材料清单中至多占一行。"
    "管线行 `数量 = Σ管段长度`，单位 `m`；设备、阀门与仪表类 `数量 = 实例计数` 且恒为 1，单位 `台/个`；"
    "管件不单独编号、继承所属管路的位号，同一宿主位号下的多个实例合并为一行。"
    "本不变量与 `DR-004` 不冲突：清单不聚合不同位号，只对继承同一宿主位号的管件求和。"
    "`位号唯一校验` 关闭时仍按 `(类型编码, 位号)` 一行，同类重复以 `DR-004` 错误呈现。\n\n"
    + BOM_TABLE
)

# REQ-DELV-001 owns the aggregation key, so its acceptance criteria must state
# the type-scoped key rather than the rejected "3 个 V-1001 实例聚合为 1 行".
BOM_AGGREGATION_ENTRY_TEXT = (
    "### REQ-DELV-001 材料清单聚合键与唯一性不变量\n\n"
    "- **优先级**：必须\n"
    "- **用户故事**：作为设计人员，我希望材料清单以复合键聚合，以便清单与主数据一一对应。\n"
    "- **验收标准**：\n"
    "  1. 聚合键为 `(类型编码, 位号)`：设备 `P-1001` 与阀门 `P-1001` 分属两个类型编码，各占 1 行。\n"
    "  2. 同一 `(类型编码, 位号)` 出现 2 行时返回 `BOM_DUPLICATE_TAG`；不同 `类型编码` 的同值位号不触发。\n"
    "- **数据与界面契约**：\n"
    "  | 字段 | 类型 | 必填 | 约束 |\n"
    "  | --- | --- | --- | --- |\n"
    "  | 聚合键 | String | 是 | 恒为 `(类型编码, 位号)` 复合键 |\n"
    "- **来源**：新增设计\n"
)


def _deliverable_document() -> str:
    """A synthetic 60 document that satisfies every VAL-REQ-009 check."""
    extras = {
        "材料清单聚合": BOM_BODY + "\n",
        "图纸输出": "支持 DWG、DXF、PDF、SVG 四种格式。\n\n",
        "导出矩阵": EXPORT_TABLE + "\n",
    }
    parts = ["# 交付与发布需求\n\n共 %d 条需求。\n\n" % len(DELIVERABLE_SECTIONS)]
    for index, name in enumerate(DELIVERABLE_SECTIONS, start=1):
        parts.append("## %s\n\n" % name)
        parts.append(extras.get(name, ""))
        if name == "材料清单聚合":
            parts.append(BOM_AGGREGATION_ENTRY_TEXT)
        else:
            parts.append(_entry("REQ-DELV-%03d" % index, title="%s示例" % name))
        parts.append("\n")
    return "".join(parts)


def test_traceability_deliverable_validator_accepts_the_synthetic_document():
    """Positive control: the synthetic document satisfies every VAL-REQ-009 check."""
    errors = validate_deliverable_coverage({DELIVERABLE_PATH: _deliverable_document()})
    assert errors == [], "the synthetic deliverable document must pass: %s" % errors


def test_traceability_deliverable_document_satisfies_the_contract():
    """Real document: five sections, the invariant, the formulas, four formats."""
    errors = validate_deliverable_coverage(load_documents(DOCS))
    assert errors == [], "VAL-REQ-009 errors: %s" % errors


def test_traceability_deliverable_document_meets_the_id_floor():
    """Real document: at least five REQ-DELV entries."""
    text = load_documents(DOCS)[DELIVERABLE_PATH]
    ids = {entry["id"] for entry in split_entries(text)}
    assert len(ids) >= 5, "the deliverable document defines %d requirements" % len(ids)


def test_traceability_deliverable_validator_rejects_a_weakened_bom_invariant():
    """Negative control: the uniqueness invariant is a sentence, not a theme."""
    document = _deliverable_document().replace(
        "一个位号在同一份材料清单中至多占一行",
        "一个位号可以在同一份材料清单中出现多行",
    )
    errors = validate_deliverable_coverage({DELIVERABLE_PATH: document})
    assert any("does not state the invariant" in error for error in errors), errors


def test_traceability_deliverable_validator_rejects_a_missing_quantity_formula():
    """Negative control: dropping the length summation must name the term."""
    document = _deliverable_document().replace(
        "`数量 = Σ管段长度`", "`数量 = 管段长度合计`"
    )
    assert "Σ" not in document, "the control removed the summation sign"
    errors = validate_deliverable_coverage({DELIVERABLE_PATH: document})
    assert any("quantity term 'Σ'" in error for error in errors), errors


def test_traceability_deliverable_validator_rejects_a_missing_drawing_format():
    """Negative control: all four drawing formats are load-bearing."""
    document = _deliverable_document().replace(
        "支持 DWG、DXF、PDF、SVG 四种格式。", "支持 DWG、DXF、PDF 三种格式。"
    )
    errors = validate_deliverable_coverage({DELIVERABLE_PATH: document})
    assert any("does not cover the SVG format" in error for error in errors), errors


def test_traceability_deliverable_validator_rejects_a_thin_export_matrix():
    """Negative control: four formats is below the six-format floor."""
    document = _deliverable_document().replace(
        EXPORT_TABLE,
        "\n".join(EXPORT_TABLE.strip().splitlines()[:6]) + "\n",
    )
    assert "JSON" not in document, "the control kept the full matrix"
    errors = validate_deliverable_coverage({DELIVERABLE_PATH: document})
    assert any("at least 6 required" in error for error in errors), errors


def test_traceability_deliverable_validator_rejects_a_missing_bom_column():
    """Negative control: the six standard BOM columns are the prototype's header."""
    document = _deliverable_document().replace(
        "| 序号 | 类型 | 位号 | 名称 | 数量 | 单位 |",
        "| 序号 | 类型 | 位号 | 名称 | 数量 |",
    ).replace(
        "| --- | --- | --- | --- | --- | --- |",
        "| --- | --- | --- | --- | --- |",
    )
    errors = validate_deliverable_coverage({DELIVERABLE_PATH: document})
    assert any("no standard header" in error for error in errors), errors


def test_traceability_deliverable_validator_rejects_a_missing_document():
    """Negative control: an absent area document is reported, not silently skipped."""
    errors = validate_deliverable_coverage({})
    assert errors == [
        "%s: deliverables and publication requirement document is missing"
        % DELIVERABLE_PATH
    ]


def test_traceability_deliverable_validator_rejects_the_retired_quantity_category():
    """Negative control: 管路类 must not come back into the quantity contract.

    60:7 states 管路 does not form its own row, so a contract cell that says
    管路类 describes a category the material list never emits.
    """
    document = load_documents(DOCS)[DELIVERABLE_PATH]
    assert "管路类" not in document, "the delivered document still names 管路类"
    degraded = document.replace(
        "| 数量 | Number | 是 | 管线行保留 2 位小数 |",
        "| 数量 | Number | 是 | 管路类保留 2 位小数 |",
    )
    assert "管路类" in degraded, "the control changed nothing"
    errors = validate_deliverable_coverage({DELIVERABLE_PATH: degraded})
    assert any("still names '管路类'" in error for error in errors), errors


BOM_TWO_SIDED_ROWS = [
    {
        "序号": "1",
        "类型": "设备",
        "位号": "P-1001",
        "名称": "离心泵",
        "数量": "1",
        "单位": "台",
    },
    {
        "序号": "2",
        "类型": "阀门",
        "位号": "V-1001",
        "名称": "截止阀 DN50",
        "数量": "2",
        "单位": "个",
    },
    {
        "序号": "3",
        "类型": "管件",
        "位号": "L-1001-01",
        "名称": "90°弯头 DN50",
        "数量": "3",
        "单位": "个",
    },
]


def test_traceability_bom_rows_reject_a_multi_instance_independent_tag():
    """The failing half: 2 instances under one 阀门 位号 is exactly what DR-004 bans."""
    bad = [row for row in BOM_TWO_SIDED_ROWS if row["类型"] == "阀门"][0]
    assert float(bad["数量"]) > 1, "the control row must really count more than one"
    errors = validate_bom_rows(BOM_TWO_SIDED_ROWS)
    assert any(
        "V-1001" in error and "keeps 数量 1" in error for error in errors
    ), errors
    # The violation is scoped to the 阀门 row: the 设备 row is not reported.
    assert not any("P-1001" in error for error in errors), errors


def test_traceability_bom_rows_accept_a_multi_instance_inherited_tag():
    """The passing half: 管件 inherit the host 位号, so 数量 3 is legal."""
    good = [row for row in BOM_TWO_SIDED_ROWS if row["类型"] == "管件"][0]
    assert float(good["数量"]) > 1, "the control row must really count more than one"
    without_the_violation = [
        row for row in BOM_TWO_SIDED_ROWS if row["类型"] != "阀门"
    ]
    assert len(without_the_violation) == 2, "the control still carries the bad row"
    assert validate_bom_rows(without_the_violation) == []


def test_traceability_bom_rows_reject_a_duplicate_composite_key_across_two_rows():
    """The row-key half: the same (类型编码, 位号) twice breaks the invariant."""
    rows = [
        dict(BOM_TWO_SIDED_ROWS[0]),
        {
            "序号": "2",
            "类型": "设备",
            "位号": "P-1001",
            "名称": "备用泵",
            "数量": "1",
            "单位": "台",
        },
    ]
    assert rows[0]["位号"] == rows[1]["位号"], "the control did not duplicate the tag"
    assert rows[0]["类型"] == rows[1]["类型"], "the control changed the type"
    errors = validate_bom_rows(rows)
    assert any(
        "repeats the (类型编码, 位号) of row 1" in error and "at most one row" in error
        for error in errors
    ), errors


def test_traceability_bom_rows_accept_the_same_tag_under_two_types():
    """The passing half of the two-sided control: 设备 P-1001 + 阀门 P-1001 is legal.

    50-data-and-rule-engine.md:19 allows a same-value 位号 across types (its own
    example is a 管线 L-1001 and an 仪表回路 L-1001) and DR-004 only counts
    same-type repeats, so keying the duplicate check on 位号 alone would reject a
    project state the documents declare legal.
    """
    rows = [
        {
            "序号": "1",
            "类型": "设备",
            "位号": "P-1001",
            "名称": "离心泵",
            "数量": "1",
            "单位": "台",
        },
        {
            "序号": "2",
            "类型": "阀门",
            "位号": "P-1001",
            "名称": "截止阀 DN50",
            "数量": "1",
            "单位": "个",
        },
    ]
    assert rows[0]["位号"] == rows[1]["位号"], "the control did not share the tag value"
    assert rows[0]["类型"] != rows[1]["类型"], "the control did not differ in type"
    assert validate_bom_rows(rows) == []


def test_traceability_bom_rows_reject_an_unknown_type_word():
    """Negative control: a typo'd 类型编码 must be reported, not skipped.

    ``阀们`` matches neither the independently-tagged rule nor the inherited-tag
    rule, so before this branch existed both rules fell through and the row
    returned ``[]``. A guard that only holds for well-typed input proves
    nothing about the input it is there to catch.
    """
    rows = [
        {
            "序号": "1",
            "类型": "阀们",
            "位号": "V-1",
            "名称": "截止阀 DN50",
            "数量": "9",
            "单位": "个",
        }
    ]
    errors = validate_bom_rows(rows)
    assert errors, "a typo'd 类型编码 passed silently"
    assert any("阀们" in error for error in errors), errors
    assert any("only carries" in error for error in errors), errors


def test_traceability_bom_rows_reject_a_non_finite_quantity():
    """Negative control: ``nan``/``inf`` 数量 must be a row error, not a raise.

    ``int(nan)`` raised ``ValueError: cannot convert float NaN to integer`` out
    of the validator, so one malformed cell aborted the whole material-list
    check instead of naming the row.
    """
    for value in ("nan", "inf", "-inf", float("nan"), float("inf")):
        rows = [
            {
                "序号": "1",
                "类型": "管件",
                "位号": "L-1-01",
                "名称": "90°弯头 DN50",
                "数量": value,
                "单位": "个",
            }
        ]
        errors = validate_bom_rows(rows)
        assert errors, "a non-finite 数量 %r was accepted" % (value,)
        assert any("non-finite" in error for error in errors), errors


def test_traceability_bom_rows_accept_every_known_type_word():
    """Positive control: the five legal type words still pass unchanged."""
    rows = [
        {"序号": "1", "类型": "设备", "位号": "P-1001", "名称": "离心泵", "数量": "1", "单位": "台"},
        {"序号": "2", "类型": "阀门", "位号": "V-1001", "名称": "截止阀", "数量": "1", "单位": "个"},
        {"序号": "3", "类型": "仪表", "位号": "TI-1001", "名称": "温度变送器", "数量": "1", "单位": "台"},
        {"序号": "4", "类型": "管线", "位号": "L-1001", "名称": "进料管线", "数量": "24.5", "单位": "m"},
        {"序号": "5", "类型": "管件", "位号": "L-1001-01", "名称": "弯头", "数量": "3", "单位": "个"},
    ]
    assert validate_bom_rows(rows) == []


def test_traceability_bom_document_obeys_the_row_identity_ruling():
    """Real document: the ruling prose is present and its sample table is legal."""
    text = load_documents(DOCS)[DELIVERABLE_PATH]
    assert validate_bom_row_identity(text) == []
    table = find_bom_header(section_text(text, "材料清单聚合"))
    assert table is not None, "the sample table lost its standard header"
    parsed = bom_rows(table)
    assert parsed, "the sample table has no data row"
    assert any(row["类型"] == "管件" and float(row["数量"]) > 1 for row in parsed), (
        "the sample no longer shows a legal multi-instance 管件 row"
    )
    for row in parsed:
        if row["类型"] in ("设备", "阀门", "仪表"):
            assert float(row["数量"]) == 1, (
                "row %s counts %s under an independent 位号"
                % (row["位号"], row["数量"])
            )


def test_traceability_bom_document_validator_rejects_a_merged_independent_tag():
    """Negative control: putting two instances under V-1001 must be caught."""
    document = _deliverable_document().replace(
        "| 3 | 阀门 | V-1001 | 截止阀 DN50 | 1 | 个 |",
        "| 3 | 阀门 | V-1001 | 截止阀 DN50 | 2 | 个 |",
    )
    assert "| 2 | 个 |" in document, "the control changed nothing"
    errors = validate_bom_row_identity(document)
    assert any(
        "V-1001" in error and "keeps 数量 1" in error for error in errors
    ), errors


def test_traceability_bom_document_validator_rejects_a_missing_ruling():
    """Negative control: the DR-004 compatibility argument is load-bearing."""
    document = _deliverable_document().replace("本不变量与 `DR-004` 不冲突", "本不变量与 `DR-004` 相容")
    assert "不冲突" not in document, "the control kept the ruling"
    errors = validate_bom_row_identity(document)
    assert any("不冲突" in error for error in errors), errors


def test_traceability_tag_master_key_is_composite():
    """The master-data key is (类型编码, 位号), matching same-type uniqueness."""
    text = load_documents(DOCS)[DATA_PATH]
    assert validate_tag_master_key(text) == []


def test_traceability_tag_master_key_validator_rejects_a_single_column_key():
    """Negative control: restoring the single-column key must be caught."""
    text = load_documents(DOCS)[DATA_PATH]
    degraded = text.replace(
        "`(类型编码, 位号)` 是该实例在数据层的唯一键",
        "位号是该实例在数据层的唯一主键",
    )
    assert degraded != text, "the control changed nothing"
    errors = validate_tag_master_key(degraded)
    assert errors, "the degraded key was accepted"
    assert any("唯一主键" in error for error in errors), errors


def test_traceability_tag_master_key_validator_rejects_the_uniqueness_paragraph_alone():
    """Negative control: 50:19 alone reverted to '位号是唯一键' must turn red.

    The composite key is stated in two places (the opening paragraph and the
    uniqueness paragraph). Reverting only the second one used to leave the
    validator green, because the phrase was still present in the first. Both
    guards must fire: the single-column phrase table and the scope paragraph.
    """
    text = load_documents(DOCS)[DATA_PATH]
    original = (
        "唯一性在项目级生效，唯一键为 `(类型编码, 位号)` 复合键：同一项目内、跨全部图纸、"
        "同一类型下的两个实例不得登记相同位号；不同类型对象允许同值位号，"
        "例如管线 `L-1001` 与仪表回路 `L-1001` 分属不同类型互不影响。"
        "`位号唯一校验` 开关关闭时不做该验证。"
    )
    assert original in text, "the uniqueness paragraph changed; update this control"
    degraded = text.replace(original, "唯一性在项目级生效，位号是唯一键。")
    assert degraded != text, "the control changed nothing"
    errors = validate_tag_master_key(degraded)
    assert any("位号是唯一键" in error for error in errors), errors
    assert any("paragraph does not state the composite key" in error for error in errors), errors


def test_traceability_tag_master_key_validator_rejects_a_bare_primary_key_field_constraint():
    """Negative control: the field-table constraint '≤ 32 字符，主键' must turn red.

    A bare 主键 is the single-column wording the composite key replaced; only
    the qualified forms (复合键 / 唯一键 / 组成) stay legal.
    """
    text = load_documents(DOCS)[DATA_PATH]
    degraded = text.replace(
        "≤ 32 字符，与类型编码组成唯一键", "≤ 32 字符，主键"
    )
    assert degraded != text, "the control changed nothing"
    errors = validate_tag_master_key(degraded)
    assert any("bare 主键" in error for error in errors), errors


def test_traceability_bom_row_identity_validator_rejects_the_rejected_aggregation_wording():
    """Negative control: '3 个 V-1001 实例聚合为 1 行' is the round-1 rejected state.

    REQ-DELV-001's acceptance criteria must state the type-scoped key, so
    restoring the merged-instance wording fails even though the section prose
    still mentions 类型编码 elsewhere.
    """
    document = _deliverable_document().replace(
        "设备 `P-1001` 与阀门 `P-1001` 分属两个类型编码，各占 1 行。",
        "3 个 `V-1001` 实例聚合为 1 行。",
    )
    assert "3 个 `V-1001` 实例聚合为 1 行" in document, "the control changed nothing"
    errors = validate_bom_row_identity(document)
    assert any(
        "REQ-DELV-001" in error and "各占 1 行" in error for error in errors
    ), errors


def test_traceability_bom_row_identity_validator_rejects_a_missing_composite_row_key():
    """Negative control: the section prose must name the composite row key."""
    document = _deliverable_document().replace(
        "材料清单的行键是位号主数据使用的复合键 `(类型编码, 位号)`",
        "材料清单的行键是位号",
    )
    assert "复合键 `(类型编码, 位号)`" not in document, "the control changed nothing"
    errors = validate_bom_row_identity(document)
    assert any(
        "composite row key" in error or "材料清单的行键是位号主数据使用的复合键" in error
        for error in errors
    ), errors


def test_traceability_bom_rows_parser_reports_a_short_row_instead_of_skipping_it():
    """Negative control: a malformed sample row is an error, never a silent skip."""
    table = find_bom_header(section_text(_deliverable_document(), "材料清单聚合"))
    assert table is not None, "the synthetic document lost its standard header"
    short = dict(table)
    short["rows"] = list(table["rows"]) + [["6", "设备", "P-1001"]]
    collected = []
    kept = bom_rows(short, collected)
    assert len(kept) == len(table["rows"]), "the short row was silently kept"
    assert any("expected 6" in error for error in collected), collected
    with pytest.raises(ValueError):
        bom_rows(short)


# ---------------------------------------------------------------------------
# Collaboration, import/export, migration and the reuse mapping
# ---------------------------------------------------------------------------


def _collaboration_document() -> str:
    chain_table = (
        "| 角色权限 | PBS 节点 ACL | 图纸状态 | 判定 | 拒绝级别 |\n"
        "| --- | --- | --- | --- | --- |\n"
        "| 允许 | 允许 | 单人编辑 | 允许 | — |\n"
        "| 允许 | 允许 | 只读 | 拒绝 | 图纸状态 |\n"
        "| 允许 | 拒绝 | 协同编辑 | 拒绝 | PBS 节点 ACL |\n"
        "| 拒绝 | 允许 | 单人编辑 | 拒绝 | 角色权限 |\n"
        "| 允许 | 允许 | 协同编辑 | 允许 | — |\n"
    )
    state_table = (
        "| 图纸状态 | 含义 | 可写 | 允许的编辑会话数 |\n"
        "| --- | --- | --- | --- |\n"
        "| 单人编辑 | 单人独占编辑 | 是 | 1 |\n"
        "| 协同编辑 | 多人共享编辑 | 是 | 多 |\n"
        "| 只读 | 只读查看 | 否 | 0 |\n"
    )
    concurrency_table = (
        "| 状态 | 事件 | 动作 |\n"
        "| --- | --- | --- |\n"
        "| 单人编辑 | 位号占用申请 | 授予占用并广播变更 |\n"
        "| 单人编辑 | 占用超时释放 | 释放占用并写入审计 |\n"
        "| 单人编辑 | 图纸被他人锁定 | 降级为只读并拒绝写入 |\n"
        "| 协同编辑 | 属性合并冲突 | 生成冲突条目并等待人工裁决 |\n"
        "| 协同编辑 | 删除已被引用对象 | 拒绝删除并列出引用方清单 |\n"
        "| 只读 | 发布后修改 | 拒绝写入，须新建修订版本 |\n"
    )
    matrix = (
        "| 角色 | 查看图纸 | 编辑图纸 | 导出图纸 | 删除图纸 | 发布签核 | 授权配置 | 规则检查 | 主数据维护 |\n"
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- |\n"
        "| 系统管理员 | 允许 | 允许 | 允许 | 允许 | 允许 | 允许 | 允许 | 允许 |\n"
        "| 项目管理员 | 允许 | 允许 | 允许 | 拒绝 | 允许 | 允许 | 允许 | 允许 |\n"
        "| 工艺设计人员 | 允许 | 允许 | 允许 | 拒绝 | 拒绝 | 拒绝 | 允许 | 拒绝 |\n"
        "| 校对人员 | 允许 | 允许 | 允许 | 拒绝 | 拒绝 | 拒绝 | 允许 | 拒绝 |\n"
        "| 只读查看者 | 允许 | 拒绝 | 拒绝 | 拒绝 | 拒绝 | 拒绝 | 允许 | 拒绝 |\n"
    )
    return (
        "# 协同与权限需求\n\n"
        "本文档定义协同与权限需求。权限字段名 角色、PBS 节点、图纸状态、允许、拒绝 以本文档为权威定义处。\n\n"
        "## 三级授权链\n\n"
        "判定顺序恰 3 级：角色权限、PBS 节点 ACL、图纸状态；"
        "判定 = 角色权限 × PBS 节点 ACL × 图纸状态。"
        "图纸权限的生命周期入口是 `REQ-DRW-007`，本节补齐其粒度。\n\n"
        + chain_table
        + "\n"
        + _entry("REQ-COLL-001", source="新增设计", title="三级授权链判定顺序")
        + "\n## 图纸状态\n\n图纸状态枚举恰 3 值。\n\n"
        + state_table
        + "\n"
        + _entry("REQ-COLL-002", title="图纸状态枚举与迁移")
        + "\n## 并发编辑策略\n\n同一图纸最多 1 个单人编辑会话。\n\n"
        + concurrency_table
        + "\n"
        + _entry("REQ-COLL-003", title="状态与事件的并发处置")
        + "\n## 冲突检测与解决\n\n冲突分对象级与属性级，字段级自动合并。\n\n"
        + _entry("REQ-COLL-004", title="属性合并冲突检测")
        + "\n## 编辑会话\n\n会话每 15 秒心跳一次，超时阈值 90 秒。\n\n"
        + _entry("REQ-COLL-005", title="编辑会话心跳与接管")
        + "\n## 权限矩阵\n\n本节不复刻手册编号，按角色 × 操作自定义矩阵。\n\n"
        + matrix
        + "\n"
        + _entry("REQ-COLL-006", source="新增设计", title="角色与操作二维权限矩阵")
    )


def _import_export_document() -> str:
    matrix = (
        "| 数据类别 | 导入 | 导出 | 支持程度 | 限制 |\n"
        "| --- | --- | --- | --- | --- |\n"
        "| 图纸 | DWG、DXF、SVG | DWG、DXF、PDF、SVG | 完整支持 | 图纸文件名唯一 |\n"
        "| P&ID 对象与属性 | Excel（XLSX）、CSV、JSON | Excel（XLSX）、CSV、JSON | 完整支持 | 位号必须存在 |\n"
        "| 项目主数据 | Excel（XLSX）、CSV | Excel（XLSX）、CSV | 完整支持 | 名称列非空 |\n"
        "| 配置包 | JSON | JSON | 部分支持 | 授权校验通过才导入 |\n"
        "| 报表与材料清单 | 不支持 | Excel（XLSX）、CSV、PDF | 完整支持 | 位号唯一 |\n"
        "| 符号库 | JSON | JSON、SVG | 部分支持 | 类型库须一致 |\n"
        "| 类型库与规则 | Excel（XLSX）、JSON | Excel（XLSX）、JSON | 部分支持 | 编码不可重复 |\n"
    )
    format_table = (
        "| 格式 | 导入方向 | 导出方向 | 支持程度 | 限制 |\n"
        "| --- | --- | --- | --- | --- |\n"
        "| DWG | 图纸 | 图纸 | 完整支持 | 版本 R14 至 2018 |\n"
        "| DXF | 图纸 | 图纸 | 完整支持 | 不保留语义扩展数据 |\n"
        "| PDF | 不支持 | 图纸、报表 | 完整支持 | 只读 |\n"
        "| SVG | 图纸 | 图纸 | 部分支持 | 仅当前图纸 |\n"
        "| Excel（XLSX） | 属性表、主数据 | 属性表、报表 | 完整支持 | 列映射由模板声明 |\n"
        "| CSV | 属性表、主数据 | 属性表、报表 | 完整支持 | UTF-8 编码 |\n"
        "| JSON | 配置包、符号库 | 配置包、属性表 | 完整支持 | 含 schemaVersion |\n"
    )
    return (
        "# 导入导出矩阵需求\n\n本文档定义导入导出能力矩阵。\n\n"
        "## 导入导出矩阵\n\n矩阵按数据类别给出导入与导出格式。\n\n"
        + matrix
        + "\n"
        + _entry("REQ-IO-001", source="新增设计", title="导入导出矩阵的数据类别覆盖")
        + "\n"
        + _entry("REQ-IO-002", source="§6.2.2.1.2.1.2", title="图纸导入格式与文件名唯一")
        + "\n## 格式支持与限制\n\n每种格式给出导入导出方向与限制。\n\n"
        + format_table
        + "\n"
        + _entry("REQ-IO-003", source="§6.2.2.4.1", title="图纸导出四格式")
        + "\n"
        + _entry("REQ-IO-004", source="§6.2.2.3.6", title="属性表格导入导出与行级错误定位")
    )


def _migration_document() -> str:
    confidence = (
        "| 识别置信度等级 | 置信度区间 | 判定依据 | 处置 |\n"
        "| --- | --- | --- | --- |\n"
        "| 高置信 | ≥ 0.90 | 块字典命中且位号配对唯一 | 自动接受 |\n"
        "| 中置信 | 0.60–0.89 | 形状哈希命中但配对存在多解 | 进入人工确认队列 |\n"
        "| 低置信 | < 0.60 | 仅几何相似或位号缺失 | 默认不导入 |\n"
    )
    exchange = (
        "| 标准 | 是否在范围内 | 理由 | 后续阶段位置 |\n"
        "| --- | --- | --- | --- |\n"
        "| DEXPI | 范围外 | 需要语义类树与标准类库的双向映射，工作量同量级 | P4 之后的扩展阶段 |\n"
        "| ISO 15926 | 范围外 | 需要本体映射与参考数据校验，超出本期范围 | P4 迁移与互操作 |\n"
        "| CFIHOS | 范围外 | 需要业主交付规范与版本对齐，取决于客户项目 | P4 迁移与互操作 |\n"
    )
    return (
        "# 迁移与互操作需求\n\n本文档定义迁移与互操作需求。\n\n"
        "## 存量图纸导入\n\n识别置信度分 3 级，中低置信进入人工确认队列。\n\n"
        + confidence
        + "\n人工确认流程共 6 步。\n\n"
        + _entry("REQ-MIG-001", source="§6.2.2.1.2.1.2", title="存量图纸识别的置信度分级")
        + "\n## 参考产品迁移\n\n迁移 5 类对象，按批次号幂等重跑。\n\n"
        + _entry("REQ-MIG-002", source="§5.1.13.2.2.2.2", title="迁移的字段映射与幂等重跑")
        + "\n## 三维模型关联\n\n关联键恰为位号（Tag），一对一匹配。\n\n"
        + _entry("REQ-MIG-003", source="§3.2.1", title="以位号为关联键的三维模型关联")
        + "\n## 自动化接口\n\n三条通道：`--serve`、`--http`、`--mcp`，各 1 条用例通过。\n\n"
        + _entry("REQ-MIG-004", source="新增设计", title="自动化接口三通道契约")
        + "\n## 插件扩展\n\n清单 `plugin.toml` 声明 `api_version` 13。\n\n"
        + _entry("REQ-MIG-005", source="新增设计", title="插件扩展点与 XDATA 持久化")
        + "\n## 数据交换标准\n\n本期只定义交换接口形状，三套标准的数据模型映射不在范围内。\n\n"
        + exchange
        + "\n"
        + _entry("REQ-MIG-006", source="新增设计", title="数据交换标准的范围声明")
    )


def _reuse_mapping_document(paths) -> str:
    rows = "\n".join(
        "| 实现 %d | `%s` | 复用于第 %d 项能力 | 需补语义字段与版本号 |"
        % (number, path, number)
        for number, path in enumerate(paths, start=1)
    )
    return (
        "# 复用映射\n\n本文档只引用需求 ID，不定义需求。\n\n"
        "## 复用映射表\n\n"
        "| 现有实现 | 路径 | 复用于 | 需改造点 |\n"
        "| --- | --- | --- | --- |\n" + rows + "\n\n"
        "## 改造点说明\n\n五类改造点按行分组说明。\n"
    )


def _real_reuse_paths(count: int = 12) -> list:
    from conftest import REPO_ROOT

    candidates = [
        "src/io/pid_legend/",
        "src/io/pid_legend/pairing.rs",
        "src/io/pid_legend/export.rs",
        "assets/pid-legend.json",
        "src/io/pid_pipes.rs",
        "src/io/pid_view_filter.rs",
        "src/app/commands/mod.rs",
        "src/mcp.rs",
        "src/app/automation.rs",
        "src/rest.rs",
        "crates/ocs_plugin_api",
        "docs/plugin-template-v2/plugin.toml",
        "crates/ocs_save/src/lib.rs",
        "src/io/svg_export",
        "src/io/pdf_export",
        "src/io/edit_lock.rs",
    ]
    existing = [path for path in candidates if (REPO_ROOT / path.rstrip("/")).exists()]
    assert len(existing) >= count, "fewer than %d reuse paths exist: %s" % (
        count,
        existing,
    )
    return existing[:count]


def test_traceability_collaboration_document_satisfies_the_contract():
    """VAL-REQ-010 over the real 70-collaboration-and-permissions.md."""
    documents = load_documents(DOCS)
    errors = validate_collaboration_coverage(documents)
    assert errors == [], "collaboration document breaches the contract: %s" % errors


def test_traceability_collaboration_document_defines_sixteen_entries():
    """The document states 16 entries and actually defines 16.

    Twelve until 2026-11-12; REQ-COLL-013 to 016 were added with the first
    revision against the code that exists (submission and broadcast, queueing
    and replay, drawing alignment at join, identity and deployment).
    """
    text = load_documents(DOCS)[COLLABORATION_PATH]
    entries = [entry for entry in split_entries(text) if entry["area"] == "COLL"]
    assert len(entries) == 16, "expected 16 REQ-COLL entries, got %d" % len(entries)
    numbers = sorted(int(entry["number"]) for entry in entries)
    assert numbers == list(range(1, 17)), "REQ-COLL numbers skip or repeat: %s" % numbers
    assert "共 16 条" in text, "the introduction does not state the entry count"
    overview = load_documents(DOCS)[OVERVIEW_PATH]
    assert "协同与权限需求，共 16 条" in overview, (
        "00-overview.md states another count for the collaboration document"
    )


def test_traceability_collaboration_document_cross_references_the_permission_entry():
    """REQ-DRW-007 must be wired from 70, in the 三级授权链 section itself."""
    text = load_documents(DOCS)[COLLABORATION_PATH]
    assert "REQ-DRW-007" in text, "70 never mentions REQ-DRW-007"
    assert "REQ-DRW-007" in section_text(text, "三级授权链"), (
        "REQ-DRW-007 is not cited in the 三级授权链 section"
    )


def test_traceability_collaboration_validator_accepts_the_synthetic_document():
    """Positive control: the synthetic document satisfies every VAL-REQ-010 check."""
    errors = validate_collaboration_coverage({COLLABORATION_PATH: _collaboration_document()})
    assert errors == [], "the synthetic collaboration document must pass: %s" % errors


def test_traceability_collaboration_validator_rejects_a_fourth_drawing_state():
    """Negative control: the drawing-state enum is closed at three values."""
    document = _collaboration_document().replace(
        "| 只读 | 只读查看 | 否 | 0 |\n",
        "| 只读 | 只读查看 | 否 | 0 |\n| 归档 | 归档只读 | 否 | 0 |\n",
    )
    assert "归档" in document, "the control added nothing"
    errors = validate_collaboration_coverage({COLLABORATION_PATH: document})
    assert any("enum table lists" in error for error in errors), errors


def test_traceability_collaboration_validator_rejects_a_missing_event_class():
    """Negative control: dropping a row must name the uncovered event."""
    document = _collaboration_document().replace(
        "| 只读 | 发布后修改 | 拒绝写入，须新建修订版本 |\n", ""
    )
    assert "发布后修改" not in document, "the control removed nothing"
    errors = validate_collaboration_coverage({COLLABORATION_PATH: document})
    assert any("发布后修改" in error for error in errors), errors


def test_traceability_collaboration_validator_rejects_a_third_permission_value():
    """Negative control: matrix cells are only 允许 or 拒绝."""
    document = _collaboration_document().replace(
        "| 工艺设计人员 | 允许 | 允许 | 允许 | 拒绝 | 拒绝 | 拒绝 | 允许 | 拒绝 |",
        "| 工艺设计人员 | 允许 | 允许 | 部分允许 | 拒绝 | 拒绝 | 拒绝 | 允许 | 拒绝 |",
    )
    assert "部分允许" in document, "the control changed nothing"
    errors = validate_collaboration_coverage({COLLABORATION_PATH: document})
    assert any("cells are only" in error for error in errors), errors


def test_traceability_collaboration_validator_rejects_a_manual_sourced_matrix():
    """Negative control: the matrix cannot be attributed to a manual section."""
    document = _collaboration_document().replace(
        _entry("REQ-COLL-006", source="新增设计", title="角色与操作二维权限矩阵"),
        _entry("REQ-COLL-006", source="§6.1.3", title="角色与操作二维权限矩阵"),
    )
    assert "§6.1.3" in document, "the control changed nothing"
    errors = validate_collaboration_coverage({COLLABORATION_PATH: document})
    assert any("no 权限矩阵 entry cites 新增设计" in error for error in errors), errors


def test_traceability_collaboration_validator_rejects_a_missing_document():
    """Negative control: an absent area document is reported."""
    errors = validate_collaboration_coverage({})
    assert errors == [
        "%s: collaboration requirement document is missing" % COLLABORATION_PATH
    ]


# ---------------------------------------------------------------------------
# Collaboration: error-code table, implementation status, open questions
# ---------------------------------------------------------------------------


def _collaboration_addenda(
    codes=("PERM_DENIED_ROLE", "CLAIM_HELD"),
    statuses=(("REQ-COLL-001", "已实现"), ("REQ-COLL-002", "部分")),
    questions=(("位号已被占用时后来的申请怎么办", "REQ-COLL-002"),),
) -> str:
    """A small document with two entries and the three bookkeeping sections."""
    error_rows = "".join(
        "| `%s` | 写入被拒绝的应答 | 示例含义：这次写入没有生效 | 不要重试 | 403 |\n" % code
        for code in codes
    )
    status_rows = "".join(
        "| %s | %s | 判定与把关 | 没有界面 |\n" % row for row in statuses
    )
    open_rows = "".join(
        "| %d | %s | 现在被拒绝并返回占用者 | %s |\n" % (index, question, entries)
        for index, (question, entries) in enumerate(questions, start=1)
    )
    return (
        "# 协同与权限需求\n\n"
        "写入被拒绝时返回 `PERM_DENIED_ROLE` 或 `CLAIM_HELD`。\n\n"
        "## 三级授权链\n\n"
        + _entry("REQ-COLL-001", source="新增设计", title="三级授权链判定顺序")
        + "\n## 并发编辑策略\n\n后来的申请见「未决事项」第 1 条。\n\n"
        + _entry("REQ-COLL-002", source="新增设计", title="位号占用")
        + "\n## " + ERROR_TABLE_SECTION + "\n\n"
        + "| " + " | ".join(ERROR_TABLE_COLUMNS) + " |\n"
        + "| " + " | ".join(["---"] * len(ERROR_TABLE_COLUMNS)) + " |\n"
        + error_rows
        + "\n## " + STATUS_SECTION + "\n\n"
        + "| " + " | ".join(STATUS_COLUMNS) + " |\n"
        + "| " + " | ".join(["---"] * len(STATUS_COLUMNS)) + " |\n"
        + status_rows
        + "\n## " + OPEN_SECTION + "\n\n"
        + "| " + " | ".join(OPEN_COLUMNS) + " |\n"
        + "| " + " | ".join(["---"] * len(OPEN_COLUMNS)) + " |\n"
        + open_rows
    )


def test_traceability_collaboration_error_table_registers_every_code_named():
    """Every reason code the real document names has a row in 错误码表."""
    text = load_documents(DOCS)[COLLABORATION_PATH]
    codes, errors = error_table_codes(text)
    assert errors == [], "error-code table errors: %s" % errors
    named = referenced_error_codes(text)
    assert named, "the document names no reason code at all"
    missing = sorted(set(named) - set(codes))
    assert missing == [], "codes named but not registered: %s" % missing
    assert validate_collaboration_error_table(text) == []


def test_traceability_collaboration_error_table_registers_every_protocol_code():
    """Every code the protocol library defines is registered, and none twice."""
    defined = protocol_error_codes(REPO_ROOT)
    assert len(defined) >= 20, (
        "%s yielded only %d codes; the parser lost the module" % (PROTOCOL_SOURCE, len(defined))
    )
    for expected in ("PERM_DENIED_STATE", "CLAIM_HELD", "OP_SUPERSEDED", "DUPLICATE_INFLOW_CONNECTOR"):
        assert expected in defined, "%s is not read from %s" % (expected, PROTOCOL_SOURCE)
    assert len(set(defined)) == len(defined), "the protocol library repeats a code"
    text = load_documents(DOCS)[COLLABORATION_PATH]
    errors = validate_collaboration_error_table(text, protocol_codes=defined)
    assert errors == [], "protocol codes missing from the table: %s" % errors


def test_traceability_collaboration_body_states_no_http_failure_status_as_a_value():
    """Refusals are named by code; `403` / `409` as values are gone from the body."""
    text = load_documents(DOCS)[COLLABORATION_PATH]
    strays = [
        error
        for error in validate_collaboration_error_table(text)
        if "states the HTTP status" in error
    ]
    assert strays == [], strays


def test_traceability_collaboration_error_validator_accepts_the_synthetic_document():
    document = _collaboration_addenda()
    assert validate_collaboration_error_table(document, protocol_codes=["CLAIM_HELD"]) == []


def test_traceability_collaboration_error_validator_rejects_an_unregistered_code():
    """Negative control: a code named in the body with no row in the table."""
    document = _collaboration_addenda(codes=("PERM_DENIED_ROLE",))
    errors = validate_collaboration_error_table(document)
    assert any("code CLAIM_HELD is named but not registered" in error for error in errors), errors


def test_traceability_collaboration_error_validator_rejects_a_code_registered_twice():
    document = _collaboration_addenda(codes=("PERM_DENIED_ROLE", "CLAIM_HELD", "CLAIM_HELD"))
    errors = validate_collaboration_error_table(document)
    assert any("code CLAIM_HELD is registered twice" in error for error in errors), errors


def test_traceability_collaboration_error_validator_rejects_a_missing_protocol_code():
    """Negative control: the implementation defines a code the table omits."""
    document = _collaboration_addenda()
    errors = validate_collaboration_error_table(document, protocol_codes=["OP_SUPERSEDED"])
    assert any("defines OP_SUPERSEDED" in error for error in errors), errors


def test_traceability_collaboration_error_validator_rejects_an_http_status_value():
    """Negative control: the body may not go back to `409`."""
    document = _collaboration_addenda().replace(
        "写入被拒绝时返回 `PERM_DENIED_ROLE` 或 `CLAIM_HELD`。",
        "写入被拒绝时返回 `409`，码是 `PERM_DENIED_ROLE` 或 `CLAIM_HELD`。",
    )
    errors = validate_collaboration_error_table(document)
    assert any("states the HTTP status 409" in error for error in errors), errors


def test_traceability_collaboration_error_validator_rejects_a_missing_table():
    document = _collaboration_addenda().replace("## " + ERROR_TABLE_SECTION, "## 码表")
    errors = validate_collaboration_error_table(document)
    assert any("missing section '%s'" % ERROR_TABLE_SECTION in error for error in errors), errors


def test_traceability_collaboration_status_covers_every_entry_once():
    """实现现状 has one row for each of the 16 entries, with a known status."""
    text = load_documents(DOCS)[COLLABORATION_PATH]
    errors = validate_collaboration_status(text)
    assert errors == [], "implementation-status errors: %s" % errors
    table = find_table(section_text(text, STATUS_SECTION), STATUS_COLUMNS)
    assert table is not None and len(table["rows"]) == 16, (
        "expected 16 status rows, got %s" % (table and len(table["rows"]))
    )
    observed = {_plain_cell(row[1]) for row in table["rows"]}
    assert observed <= set(STATUS_VALUES), observed


def test_traceability_collaboration_status_validator_rejects_a_missing_entry():
    document = _collaboration_addenda(statuses=(("REQ-COLL-001", "已实现"),))
    errors = validate_collaboration_status(document)
    assert any("REQ-COLL-002 has no row" in error for error in errors), errors


def test_traceability_collaboration_status_validator_rejects_an_unknown_status():
    document = _collaboration_addenda(
        statuses=(("REQ-COLL-001", "已实现"), ("REQ-COLL-002", "基本完成"))
    )
    errors = validate_collaboration_status(document)
    assert any("has status '基本完成'" in error for error in errors), errors


def test_traceability_collaboration_status_validator_rejects_a_repeated_entry():
    document = _collaboration_addenda(
        statuses=(
            ("REQ-COLL-001", "已实现"),
            ("REQ-COLL-002", "部分"),
            ("REQ-COLL-002", "未实现"),
        )
    )
    errors = validate_collaboration_status(document)
    assert any("REQ-COLL-002 has two status rows" in error for error in errors), errors


def test_traceability_collaboration_open_questions_point_at_real_entries():
    """未决事项 is numbered 1..N and every row names defined requirements."""
    documents = load_documents(DOCS)
    text = documents[COLLABORATION_PATH]
    errors = validate_collaboration_open_questions(text, documents)
    assert errors == [], "open-question errors: %s" % errors
    table = find_table(section_text(text, OPEN_SECTION), OPEN_COLUMNS)
    assert table is not None and table["rows"], "未决事项 has no rows"


def test_traceability_collaboration_open_validator_accepts_the_synthetic_document():
    document = _collaboration_addenda()
    assert validate_collaboration_open_questions(
        document, {COLLABORATION_PATH: document}
    ) == []
    assert validate_collaboration_status(document) == []


def test_traceability_collaboration_open_validator_rejects_a_citation_past_the_end():
    """Negative control: the body cites a row the table does not have."""
    document = _collaboration_addenda().replace("「未决事项」第 1 条", "「未决事项」第 3 条")
    errors = validate_collaboration_open_questions(document)
    assert any("cites open question 3" in error for error in errors), errors


def test_traceability_collaboration_open_validator_rejects_an_undefined_entry():
    document = _collaboration_addenda(
        questions=(("位号已被占用时后来的申请怎么办", "REQ-COLL-099"),)
    )
    errors = validate_collaboration_open_questions(
        document, {COLLABORATION_PATH: document}
    )
    assert any("names REQ-COLL-099, which is not defined" in error for error in errors), errors


def test_traceability_collaboration_open_validator_rejects_a_misnumbered_row():
    document = _collaboration_addenda().replace(
        "| 1 | 位号已被占用时后来的申请怎么办 |", "| 2 | 位号已被占用时后来的申请怎么办 |"
    )
    errors = validate_collaboration_open_questions(document)
    assert any("is numbered '2'; expected 1" in error for error in errors), errors


def test_traceability_import_export_document_satisfies_the_contract():
    """VAL-REQ-011 over the real 80-import-export-matrix.md."""
    documents = load_documents(DOCS)
    errors = validate_import_export_coverage(documents)
    assert errors == [], "import/export document breaches the contract: %s" % errors


def test_traceability_import_export_validator_accepts_the_synthetic_document():
    """Positive control for the import/export matrix checks."""
    errors = validate_import_export_coverage({IMPORT_EXPORT_PATH: _import_export_document()})
    assert errors == [], "the synthetic import/export document must pass: %s" % errors


def test_traceability_import_export_validator_rejects_a_missing_category():
    """Negative control: the symbol-library row is one of the six required rows."""
    document = _import_export_document().replace(
        "| 符号库 | JSON | JSON、SVG | 部分支持 | 类型库须一致 |\n", ""
    )
    assert "符号库" not in document.split("## 格式支持与限制")[0], (
        "the control removed nothing from the matrix"
    )
    errors = validate_import_export_coverage({IMPORT_EXPORT_PATH: document})
    assert any("data category '符号库'" in error for error in errors), errors


def test_traceability_import_export_validator_rejects_a_missing_format():
    """Negative control: all seven formats must appear in the matrix."""
    document = _import_export_document().replace("PDF", "PostScript")
    errors = validate_import_export_coverage({IMPORT_EXPORT_PATH: document})
    assert any("format 'PDF'" in error for error in errors), errors


def test_traceability_migration_document_satisfies_the_contract():
    """VAL-REQ-012 over the real 90-migration-and-interop.md."""
    documents = load_documents(DOCS)
    errors = validate_migration_coverage(documents)
    assert errors == [], "migration document breaches the contract: %s" % errors


def test_traceability_migration_validator_accepts_the_synthetic_document():
    """Positive control for the migration checks."""
    errors = validate_migration_coverage({MIGRATION_PATH: _migration_document()})
    assert errors == [], "the synthetic migration document must pass: %s" % errors


def test_traceability_migration_validator_rejects_two_confidence_levels():
    """Negative control: at least three confidence levels are required."""
    document = _migration_document().replace(
        "| 低置信 | < 0.60 | 仅几何相似或位号缺失 | 默认不导入 |\n", ""
    )
    errors = validate_migration_coverage({MIGRATION_PATH: document})
    assert any("at least 3 are required" in error for error in errors), errors


def test_traceability_migration_validator_rejects_a_missing_automation_channel():
    """Negative control: all three automation channels are load-bearing."""
    document = _migration_document().replace("`--mcp`", "MCP 通道")
    assert "--mcp" not in document, "the control removed nothing"
    errors = validate_migration_coverage({MIGRATION_PATH: document})
    assert any("does not state '--mcp'" in error for error in errors), errors


def test_traceability_migration_validator_rejects_an_undeclared_standard():
    """Negative control: DEXPI / ISO 15926 / CFIHOS must each be declared."""
    document = _migration_document().replace(
        "| CFIHOS | 范围外 | 需要业主交付规范与版本对齐，取决于客户项目 | P4 迁移与互操作 |\n",
        "",
    )
    errors = validate_migration_coverage({MIGRATION_PATH: document})
    assert any("standard 'CFIHOS'" in error for error in errors), errors


def test_traceability_migration_validator_rejects_an_out_of_scope_without_a_phase():
    """Negative control: a 范围外 standard must name where it lands later."""
    document = _migration_document().replace(
        "| ISO 15926 | 范围外 | 需要本体映射与参考数据校验，超出本期范围 | P4 迁移与互操作 |",
        "| ISO 15926 | 范围外 | 需要本体映射与参考数据校验，超出本期范围 | — |",
    )
    errors = validate_migration_coverage({MIGRATION_PATH: document})
    assert any("names no later phase" in error for error in errors), errors


def test_traceability_reuse_mapping_document_uses_real_paths():
    """VAL-REQ-014: >=12 rows whose 路径 all exist in this repository."""
    from conftest import REPO_ROOT

    documents = load_documents(DOCS)
    errors = validate_reuse_mapping(documents, repo_root=REPO_ROOT)
    assert errors == [], "reuse mapping breaches the contract: %s" % errors


def test_traceability_reuse_mapping_document_defines_no_requirement_ids():
    """95 may cite IDs but must not define them (the parser sees no H3 entries)."""
    text = load_documents(DOCS)[REUSE_PATH]
    assert split_entries(text) == [], "95-reuse-mapping.md defines requirement entries"


def test_traceability_reuse_mapping_validator_accepts_verified_paths():
    """Positive control: twelve real paths pass the existence check."""
    from conftest import REPO_ROOT

    paths = _real_reuse_paths(12)
    errors = validate_reuse_mapping(
        {REUSE_PATH: _reuse_mapping_document(paths)}, repo_root=REPO_ROOT
    )
    assert errors == [], "twelve real paths must pass: %s" % errors


def test_traceability_reuse_mapping_validator_rejects_a_nonexistent_path():
    """Negative control: a path string that exists nowhere is reported."""
    from conftest import REPO_ROOT

    paths = _real_reuse_paths(12)
    paths[3] = "src/io/pid_legend/does-not-exist.rs"
    errors = validate_reuse_mapping(
        {REUSE_PATH: _reuse_mapping_document(paths)}, repo_root=REPO_ROOT
    )
    assert any("does not exist in the repository" in error for error in errors), errors


def test_traceability_reuse_mapping_validator_rejects_eleven_rows():
    """Negative control: the twelve-row floor is enforced."""
    from conftest import REPO_ROOT

    paths = _real_reuse_paths(12)[:11]
    errors = validate_reuse_mapping(
        {REUSE_PATH: _reuse_mapping_document(paths)}, repo_root=REPO_ROOT
    )
    assert any("at least 12 are required" in error for error in errors), errors


def test_traceability_reuse_mapping_validator_rejects_a_missing_section():
    """Negative control: the two section names are load-bearing."""
    from conftest import REPO_ROOT

    paths = _real_reuse_paths(12)
    document = _reuse_mapping_document(paths).replace("## 改造点说明", "## 说明")
    errors = validate_reuse_mapping({REUSE_PATH: document}, repo_root=REPO_ROOT)
    assert any("missing section '改造点说明'" in error for error in errors), errors


def test_traceability_reuse_mapping_requires_a_repo_root():
    """Negative control: without a repo root the path arm cannot be satisfied."""
    paths = _real_reuse_paths(12)
    errors = validate_reuse_mapping({REUSE_PATH: _reuse_mapping_document(paths)})
    assert any("needs a repo root" in error for error in errors), errors


# ---------------------------------------------------------------------------
# VAL-CROSS-003 screen-level elements.
#
# The assertion requires the screen mapped from a requirement to show that
# requirement's key interface element names. Nothing bound this环节 before, so
# typelib and symbols shipped without REQ-TYPE-001's 类型编码 field or
# REQ-SYM-001's four legend standards while the suite stayed green.
#
# These checks are a SUBSET assertion: each element name must appear somewhere
# in the prototype bundle. They do not prove that every allowed enum value is
# rendered, and they do not read the rendered DOM.
# ---------------------------------------------------------------------------

REQUIREMENT_ELEMENTS = CATALOG.parent / "requirement-elements.json"
SCREENS_BUNDLE = PROTO / "assets" / "screens.js"
OVERVIEW_PATH = "requirements/00-overview.md"


def load_requirement_elements() -> dict:
    assert REQUIREMENT_ELEMENTS.exists(), (
        "requirement-elements.json missing at " + str(REQUIREMENT_ELEMENTS)
    )
    with REQUIREMENT_ELEMENTS.open(encoding="utf-8") as fh:
        data = json.load(fh)
    assert isinstance(data.get("requirements"), dict), (
        "requirement-elements.json must carry a 'requirements' object"
    )
    return data["requirements"]


def _overview_matrix_screens(documents: dict) -> dict:
    """requirement id -> screen id or 无界面, parsed from the traceability table."""
    text = documents.get(OVERVIEW_PATH, "")
    mapping = {}
    for line in text.splitlines():
        if not line.startswith("|"):
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cells) < 3:
            continue
        match = REQ_ID_RE.search(cells[0])
        if not match:
            continue
        mapping[cells[0]] = cells[2]
    return mapping


def test_traceability_requirement_elements_reference_real_screens():
    entries = load_requirement_elements()
    valid = set(_screen_ids())
    bad = {
        req: entry.get("screen")
        for req, entry in entries.items()
        if entry.get("screen") not in valid
    }
    assert bad == {}, "requirement-elements.json names unknown screens: %s" % bad
    for req, entry in entries.items():
        assert isinstance(entry.get("elements"), list) and entry["elements"], (
            "%s has no element list" % req
        )


def test_traceability_requirement_elements_match_the_matrix():
    entries = load_requirement_elements()
    documents = load_documents(DOCS)
    matrix = _overview_matrix_screens(documents)
    problems = []
    for req, entry in entries.items():
        if req not in matrix:
            problems.append("%s is absent from the traceability matrix" % req)
            continue
        if matrix[req] != entry["screen"]:
            problems.append(
                "%s maps to %s in the matrix but %s in requirement-elements.json"
                % (req, matrix[req], entry["screen"])
            )
    assert problems == [], "; ".join(problems)


def test_traceability_requirement_elements_are_absent_for_screenless_requirements():
    entries = load_requirement_elements()
    documents = load_documents(DOCS)
    matrix = _overview_matrix_screens(documents)
    offenders = [req for req in entries if matrix.get(req) == "无界面"]
    assert offenders == [], (
        "screenless requirements must not claim screen elements: %s" % offenders
    )


def test_traceability_requirement_elements_appear_in_the_bundle():
    entries = load_requirement_elements()
    text = SCREENS_BUNDLE.read_text(encoding="utf-8")
    missing = []
    for req, entry in entries.items():
        for element in entry["elements"]:
            if element not in text:
                missing.append("%s -> %s" % (req, element))
    assert missing == [], (
        "element names absent from the prototype bundle: %s" % missing
    )


def test_traceability_requirement_element_check_rejects_a_renamed_element():
    """Negative control: the element scan must flag a name that is absent."""
    entries = load_requirement_elements()
    text = SCREENS_BUNDLE.read_text(encoding="utf-8")
    probe = "不存在的元素名ZZZ"
    assert probe not in text, "the probe string unexpectedly exists in the bundle"
    fake = {"REQ-TYPE-001": {"screen": "typelib", "elements": [probe]}}
    missing = [
        "%s -> %s" % (req, element)
        for req, entry in fake.items()
        for element in entry["elements"]
        if element not in text
    ]
    assert missing == ["REQ-TYPE-001 -> " + probe], (
        "the element scan did not flag an absent element: %s" % (missing,)
    )
    real_missing = [
        "%s -> %s" % (req, element)
        for req, entry in entries.items()
        for element in entry["elements"]
        if element not in text
    ]
    assert real_missing == [], "real element names are missing: %s" % real_missing
