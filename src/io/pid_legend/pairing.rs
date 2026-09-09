//! One-to-one pairing of tags with symbols: as many pairs as the candidates
//! allow and, of those pairings, the shortest. Both families take their
//! outside tags through it.

use std::collections::BTreeMap;

/// One-to-one pairing of texts with components over the candidate
/// `(distance, text, component)` edges: as many pairs as the edges allow
/// and, of those pairings, the least total distance. Returns the indices of
/// the chosen edges.
///
/// Successive shortest augmenting paths on the unit-capacity flow network
/// source -> texts -> components -> sink; each augmentation adds one pair at
/// the least extra distance, so after the last one the pairing is both
/// maximum and cheapest. The graph is small (a sheet's tags, each within
/// reach of a few components), so Bellman-Ford with a queue is plenty.
pub(super) fn match_pairs(edges: &[(f64, usize, usize)]) -> Vec<usize> {
    struct Arc {
        to: usize,
        cap: u8,
        cost: f64,
        rev: usize,
    }
    const SOURCE: usize = 0;
    const SINK: usize = 1;
    let mut text_node: BTreeMap<usize, usize> = BTreeMap::new();
    let mut comp_node: BTreeMap<usize, usize> = BTreeMap::new();
    for &(_, k, ci) in edges {
        text_node.entry(k).or_insert(0);
        comp_node.entry(ci).or_insert(0);
    }
    let mut n = 2;
    for node in text_node.values_mut().chain(comp_node.values_mut()) {
        *node = n;
        n += 1;
    }
    let mut adj: Vec<Vec<Arc>> = (0..n).map(|_| Vec::new()).collect();
    let add = |adj: &mut Vec<Vec<Arc>>, from: usize, to: usize, cost: f64| -> usize {
        let (fi, ti) = (adj[from].len(), adj[to].len());
        adj[from].push(Arc {
            to,
            cap: 1,
            cost,
            rev: ti,
        });
        adj[to].push(Arc {
            to: from,
            cap: 0,
            cost: -cost,
            rev: fi,
        });
        fi
    };
    for &t in text_node.values() {
        add(&mut adj, SOURCE, t, 0.0);
    }
    for &c in comp_node.values() {
        add(&mut adj, c, SINK, 0.0);
    }
    let arcs: Vec<(usize, usize)> = edges
        .iter()
        .map(|&(d, k, ci)| {
            let t = text_node[&k];
            (t, add(&mut adj, t, comp_node[&ci], d))
        })
        .collect();
    loop {
        let mut dist = vec![f64::INFINITY; n];
        let mut prev: Vec<Option<(usize, usize)>> = vec![None; n];
        let mut queued = vec![false; n];
        let mut queue = std::collections::VecDeque::from([SOURCE]);
        dist[SOURCE] = 0.0;
        while let Some(u) = queue.pop_front() {
            queued[u] = false;
            for (i, arc) in adj[u].iter().enumerate() {
                if arc.cap > 0 && dist[u] + arc.cost < dist[arc.to] - 1e-9 {
                    dist[arc.to] = dist[u] + arc.cost;
                    prev[arc.to] = Some((u, i));
                    if !queued[arc.to] {
                        queued[arc.to] = true;
                        queue.push_back(arc.to);
                    }
                }
            }
        }
        if !dist[SINK].is_finite() {
            break;
        }
        let mut v = SINK;
        while let Some((u, i)) = prev[v] {
            let rev = adj[u][i].rev;
            adj[u][i].cap -= 1;
            adj[v][rev].cap += 1;
            v = u;
        }
    }
    arcs.iter()
        .enumerate()
        .filter(|(_, &(t, i))| adj[t][i].cap == 0)
        .map(|(e, _)| e)
        .collect()
}
