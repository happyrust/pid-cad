//! 随图保存的协同状态（`REQ-COLL-015`）：三条图级 XRecord，挂在模型空间
//! block record 的扩展字典下，与 [`super::graph_data`] 的四个 key 同层。
//!
//! * [`PID_AUTHOR_COLLAB`]：一条记录，三项——图号 `drawing_id`、服务器库
//!   标识 `repo_id`、已应用到的修订 `revision`。它们说明的是「这份文件
//!   上一次对齐的是哪个库、对齐到哪儿」；下次加入时客户端按它们对账。
//!   可选的第四项 `diverged`（[`Diverged`]）说这份图已经知道自己与服务器
//!   上的不一样了，那三项也就不再作数。
//! * [`PID_AUTHOR_COLLAB_QUEUE`]：已做出但房间尚未回答的本地修改，一条
//!   op 一条记录，顺序即队列顺序。进程结束后它们不再在内存里，靠这条
//!   记录回到队列重新提交。
//! * [`PID_AUTHOR_COLLAB_UNSYNCED`]：本图里有、服务器没有、也不会再自动
//!   提交的修改（[`Unsynced`]），只增不减。
//!
//! 三条记录只在保存快照里写（`document_for_save` 之后），不动活文档：
//! 不弄脏 `dirty`，也不进撤销栈。不连服务器的图没有这几条，也不受影响
//! ——打开、绘制、保存都照常。

use document::Document;

use super::graph_data;

/// 三项协同状态的 XRecord key。
pub const PID_AUTHOR_COLLAB: &str = "PID_AUTHOR_COLLAB";

/// 未回答 op 队列的 XRecord key。
pub const PID_AUTHOR_COLLAB_QUEUE: &str = "PID_AUTHOR_COLLAB_QUEUE";

/// 未同步清单的 XRecord key。
pub const PID_AUTHOR_COLLAB_UNSYNCED: &str = "PID_AUTHOR_COLLAB_UNSYNCED";

/// [`Diverged::code`]：图里有同步记录，却做了没有排上队的语义修改。
pub const UNQUEUED_EDITS: &str = "UNQUEUED_EDITS";
/// [`Diverged::code`]：别人的修改写不进本图。
pub const REMOTE_UNAPPLIED: &str = "REMOTE_UNAPPLIED";
/// [`Diverged::code`]：收到的修订不连续，中间的没有收到。
pub const REVISION_GAP: &str = "REVISION_GAP";
/// [`Diverged::code`]：被拒的修改撤不回，或者撤不干净。
pub const ROLLBACK_FAILED: &str = "ROLLBACK_FAILED";

/// [`Unsynced::state`]：做了，没有排上队。
pub const UNQUEUED: &str = "unqueued";
/// [`Unsynced::state`]：提交过，服务器整步拒绝；或者建在被拒的一步上，跟着作废。
pub const REFUSED: &str = "refused";

/// 这份文件上一次对齐到的协同位置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileCollabState {
    /// 服务器登记的图号。
    pub drawing_id: String,
    /// 服务器库的标识（`meta.repo_id`）。
    pub repo_id: String,
    /// 已应用到的修订。
    pub revision: i64,
}

/// 这份图已经知道自己与服务器上的不一样：记录里的 `diverged`。写上之后
/// 不自动清除。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diverged {
    /// [`UNQUEUED_EDITS`]、[`REMOTE_UNAPPLIED`]、[`REVISION_GAP`]、
    /// [`ROLLBACK_FAILED`] 之一。
    pub code: String,
    pub message: String,
    /// Unix 秒。
    pub at: i64,
}

/// 未同步清单的一条：本图里做过、服务器没有收下、也不会再自动提交的一步。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Unsynced {
    /// `refused`（提交过，服务器整步拒绝）或 [`UNQUEUED`]。
    pub state: String,
    /// 只有 `refused` 的有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op_id: Option<String>,
    pub label: String,
    pub payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Unix 秒。
    pub at: i64,
    /// 服务器拒绝时给的码，`refused` 的才有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
    /// 本地是否已撤回，`refused` 的才有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rolled_back: Option<bool>,
}

/// 一条未回答的 op，按它出队重发时需要的样子。
///
/// 不直接序列化 `collab::Outgoing`：撤销栈步号（`steps`）活不过进程，
/// 持久化它只会假装还能撤回；`restored`/`sent` 是运行时状态。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct QueuedOp {
    pub op_id: String,
    pub payload: serde_json::Value,
    /// 这步做出来时本图应用到的修订：op 按它被审判。
    pub base_revision: i64,
    /// 步是放上（made / redone）还是拿下（undone）。
    pub on: bool,
    /// 它叫什么，被拒时的那句话用。
    pub label: String,
    /// 谁排的：排队时的用户号。只有排队的人自己提交它。旧版本存的条目没有
    /// 这一项。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// 什么时候排的，Unix 秒。旧版本存的条目没有这一项。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued_at: Option<i64>,
    /// 撤回这一条要做的修改，形状与 `payload` 里的变更相同，按要执行的
    /// 顺序（离线规格 §4.1）。没有这一项，这一条重开程序后撤不回（§5.4）；
    /// 旧版本存的条目也没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inverse: Option<Vec<serde_json::Value>>,
}

/// 读回的队列：读得出的条目，和读不出的那些。
#[derive(Clone, Debug, Default)]
pub struct QueueRead {
    /// 按写入顺序。
    pub ops: Vec<QueuedOp>,
    /// 读不出的条目，原样：是 JSON 但不是一条 op 的，就是那个 JSON；连
    /// JSON 都不是的，是它的原文（一个 JSON 字符串）。它们也是没提交的修改，
    /// 存盘时原样写回去（[`store_queue_keeping`]），不能就这样丢掉。
    pub unreadable: Vec<serde_json::Value>,
}

/// 写入（或清空）三项协同状态。`None` 表示这份文件不从属于任何库——
/// 记录被删掉，不留残值。
pub fn store_state(doc: &mut Document, state: Option<&FileCollabState>) {
    store_state_diverged(doc, state, None);
}

/// 同 [`store_state`]，连同 `diverged`。
pub fn store_state_diverged(
    doc: &mut Document,
    state: Option<&FileCollabState>,
    diverged: Option<&Diverged>,
) {
    let items = match state {
        Some(state) => {
            let mut item = serde_json::json!({
                "drawing_id": state.drawing_id,
                "repo_id": state.repo_id,
                "revision": state.revision,
            });
            if let Some(diverged) = diverged {
                item["diverged"] = serde_json::json!({
                    "code": diverged.code,
                    "message": diverged.message,
                    "at": diverged.at,
                });
            }
            vec![item]
        }
        None => Vec::new(),
    };
    graph_data::store(doc, PID_AUTHOR_COLLAB, &items);
}

/// 读回记录里的 `diverged`。没有记录、没有这一项（旧版本写的图）都是
/// `None`。有这一项而写得不全的，缺的部分读成空：有它就说明这份图知道
/// 自己不一致，不能因为写得不全就当成一致。
pub fn load_diverged(doc: &Document) -> Option<Diverged> {
    let items = graph_data::load(doc, PID_AUTHOR_COLLAB)?;
    let diverged = items.first()?.get("diverged")?;
    if diverged.is_null() {
        return None;
    }
    Some(Diverged {
        code: diverged["code"].as_str().unwrap_or_default().to_owned(),
        message: diverged["message"].as_str().unwrap_or_default().to_owned(),
        at: diverged["at"].as_i64().unwrap_or_default(),
    })
}

/// 整份未同步清单重写，按顺序。空清单删除记录。
pub fn store_unsynced(doc: &mut Document, entries: &[serde_json::Value]) {
    graph_data::store(doc, PID_AUTHOR_COLLAB_UNSYNCED, entries);
}

/// 读回未同步清单，按写入顺序。条目原样留着，读不懂的也一样（连 JSON
/// 都不是的是它的原文）：存盘时写回去，不丢。没有这条记录就是空的。
pub fn read_unsynced(doc: &Document) -> Vec<serde_json::Value> {
    let Some(loaded) = graph_data::load_reporting(doc, PID_AUTHOR_COLLAB_UNSYNCED) else {
        return Vec::new();
    };
    let unreadable = loaded.unreadable.into_iter().map(serde_json::Value::String);
    let mut entries = loaded.items;
    entries.extend(unreadable);
    entries
}

/// 在未同步清单末尾接上 `entries`，原有的条目不动。
pub fn append_unsynced(doc: &mut Document, entries: &[serde_json::Value]) {
    let mut list = read_unsynced(doc);
    list.extend_from_slice(entries);
    store_unsynced(doc, &list);
}

/// 读回三项协同状态。记录不存在、残缺或字段非法都返回 `None`：一份
/// 对不上的记录比没有记录更危险，宁可按「从未对齐过」处理。
pub fn load_state(doc: &Document) -> Option<FileCollabState> {
    let items = graph_data::load(doc, PID_AUTHOR_COLLAB)?;
    let item = items.first()?;
    Some(FileCollabState {
        drawing_id: item["drawing_id"].as_str()?.to_owned(),
        repo_id: item["repo_id"].as_str()?.to_owned(),
        revision: item["revision"].as_i64()?,
    })
}

/// 整队列重写：先清后填。空队列删除记录，不留空 XRecord。
pub fn store_queue(doc: &mut Document, ops: &[QueuedOp]) {
    store_queue_keeping(doc, ops, &[]);
}

/// 整队列重写，读不出的条目 `unreadable` 原样排在后面（[`QueueRead`]）。
pub fn store_queue_keeping(doc: &mut Document, ops: &[QueuedOp], unreadable: &[serde_json::Value]) {
    let items: Vec<serde_json::Value> = ops
        .iter()
        .filter_map(|op| serde_json::to_value(op).ok())
        .chain(unreadable.iter().cloned())
        .collect();
    graph_data::store(doc, PID_AUTHOR_COLLAB_QUEUE, &items);
}

/// 读回队列：按写入顺序。解析失败的一条被跳过，不拖垮整队；要知道跳过
/// 了哪些，用 [`read_queue`]。
pub fn load_queue(doc: &Document) -> Vec<QueuedOp> {
    read_queue(doc).ops
}

/// 读回队列，连同读不出的条目。
pub fn read_queue(doc: &Document) -> QueueRead {
    let Some(loaded) = graph_data::load_reporting(doc, PID_AUTHOR_COLLAB_QUEUE) else {
        return QueueRead::default();
    };
    let mut read = QueueRead::default();
    for item in loaded.items {
        match serde_json::from_value::<QueuedOp>(item.clone()) {
            Ok(op) => read.ops.push(op),
            Err(_) => read.unreadable.push(item),
        }
    }
    read.unreadable
        .extend(loaded.unreadable.into_iter().map(serde_json::Value::String));
    read
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Document {
        Document::new()
    }

    #[test]
    fn collab_state_round_trips_and_clears() {
        let mut doc = doc();
        assert_eq!(load_state(&doc), None, "无记录即未对齐过");
        let state = FileCollabState {
            drawing_id: "PID-1101".into(),
            repo_id: "r-abc".into(),
            revision: 42,
        };
        store_state(&mut doc, Some(&state));
        assert_eq!(load_state(&doc), Some(state));
        store_state(&mut doc, None);
        assert_eq!(load_state(&doc), None, "清空后记录不存在");
        assert_eq!(graph_data::item_count(&doc, PID_AUTHOR_COLLAB), 0);
    }

    #[test]
    fn collab_queue_round_trips_in_order_and_clears() {
        let mut doc = doc();
        assert!(load_queue(&doc).is_empty());
        let ops: Vec<QueuedOp> = (0..3)
            .map(|i| QueuedOp {
                op_id: format!("op-{i}"),
                payload: serde_json::json!({"kind": "set_property", "i": i}),
                base_revision: i as i64,
                on: true,
                label: format!("步骤 {i}"),
                user_id: None,
                queued_at: None,
                inverse: None,
            })
            .collect();
        store_queue(&mut doc, &ops);
        let back = load_queue(&doc);
        assert_eq!(
            back.iter().map(|op| op.op_id.as_str()).collect::<Vec<_>>(),
            ["op-0", "op-1", "op-2"]
        );
        store_queue(&mut doc, &[]);
        assert!(load_queue(&doc).is_empty());
        assert_eq!(graph_data::item_count(&doc, PID_AUTHOR_COLLAB_QUEUE), 0);
    }

    #[test]
    fn collab_state_diverged_round_trips_and_a_record_from_before_has_none() {
        let mut doc = doc();
        let state = FileCollabState {
            drawing_id: "PID-1101".into(),
            repo_id: "r-abc".into(),
            revision: 42,
        };
        store_state(&mut doc, Some(&state));
        assert_eq!(load_diverged(&doc), None, "旧版本写的记录没有这一项");

        let diverged = Diverged {
            code: REVISION_GAP.into(),
            message: "收到修订 44，本地基线是 42".into(),
            at: 1_795_000_000,
        };
        store_state_diverged(&mut doc, Some(&state), Some(&diverged));
        assert_eq!(load_state(&doc), Some(state.clone()), "三项照旧读得出");
        assert_eq!(load_diverged(&doc), Some(diverged));

        // 写得不全也还是说这份图知道自己不一致。
        let short = serde_json::json!({
            "drawing_id": "PID-1101",
            "repo_id": "r-abc",
            "revision": 42,
            "diverged": {"code": REMOTE_UNAPPLIED},
        });
        graph_data::store(&mut doc, PID_AUTHOR_COLLAB, &[short]);
        let read = load_diverged(&doc).expect("有这一项");
        assert_eq!(
            (read.code.as_str(), read.message.as_str(), read.at),
            (REMOTE_UNAPPLIED, "", 0)
        );

        store_state(&mut doc, None);
        assert_eq!(load_diverged(&doc), None);
    }

    #[test]
    fn collab_state_unsynced_round_trips_in_order_and_keeps_what_it_cannot_read() {
        let mut doc = doc();
        assert!(read_unsynced(&doc).is_empty(), "旧版本写的图没有这条记录");
        let entry = serde_json::to_value(Unsynced {
            state: UNQUEUED.into(),
            op_id: None,
            label: "改属性".into(),
            payload: serde_json::json!({"kind": "set_property"}),
            user_id: Some("u1".into()),
            at: 1_795_000_000,
            code: None,
            message: "本次运行没有配置协同".into(),
            rolled_back: None,
        })
        .expect("一条未同步的修改");
        assert_eq!(entry.get("op_id"), None, "没排上队的没有 op_id");
        let later = serde_json::json!({"state": "from-a-later-version"});
        store_unsynced(&mut doc, &[entry.clone(), later.clone()]);
        assert_eq!(read_unsynced(&doc), [entry, later]);

        store_unsynced(&mut doc, &[]);
        assert!(read_unsynced(&doc).is_empty());
        assert_eq!(graph_data::item_count(&doc, PID_AUTHOR_COLLAB_UNSYNCED), 0);
    }
}
