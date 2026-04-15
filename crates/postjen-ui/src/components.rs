use leptos::*;
use leptos_router::*;
use std::collections::HashMap;
use crate::api;

#[component]
pub fn StatusBadge(status: String) -> impl IntoView {
    let (icon, class) = match status.as_str() {
        "success" => ("✓", "status-success"),
        "failed" => ("✗", "status-failed"),
        "running" => ("●", "status-running"),
        "queued" => ("○", "status-queued"),
        "timed_out" => ("⏱", "status-timed_out"),
        "canceled" => ("⊘", "status-canceled"),
        "skipped" => ("→", "status-skipped"),
        "pending" => ("○", "status-pending"),
        _ => ("?", ""),
    };
    view! {
        <span class=format!("status {class}")>
            <span class="dot">{icon}</span>
            " "
            {status}
        </span>
    }
}

#[component]
pub fn Loading() -> impl IntoView {
    view! { <div class="loading">"Loading..."</div> }
}

/// パラメータ入力モーダルダイアログ
#[component]
pub fn ParamDialog(
    job_id: String,
    params: Vec<api::ParamDefinition>,
    #[prop(into)] on_close: Callback<()>,
) -> impl IntoView {
    let param_values: Vec<(String, RwSignal<String>, bool)> = params
        .iter()
        .map(|p| {
            let initial = p.default.clone().unwrap_or_default();
            (p.name.clone(), create_rw_signal(initial), p.required)
        })
        .collect();

    let (error, set_error) = create_signal(Option::<String>::None);
    let (submitting, set_submitting) = create_signal(false);

    let values_for_submit = param_values.clone();
    let job_id_submit = job_id.clone();

    let on_submit = move |_| {
        let mut map = HashMap::new();
        for (name, sig, _) in &values_for_submit {
            let v = sig.get();
            if !v.is_empty() {
                map.insert(name.clone(), v);
            }
        }
        let jid = job_id_submit.clone();
        set_submitting.set(true);
        set_error.set(None);
        spawn_local(async move {
            match api::start_run(&jid, Some(map)).await {
                Ok(resp) => {
                    let navigate = use_navigate();
                    navigate(&format!("/runs/{}", resp.run_id), Default::default());
                }
                Err(e) => {
                    set_submitting.set(false);
                    set_error.set(Some(e));
                }
            }
        });
    };

    let on_close_click = move |_| {
        on_close.call(());
    };

    view! {
        <div class="modal-overlay" on:click=on_close_click>
            <div class="modal" on:click=|e| e.stop_propagation()>
                <div class="modal-header">
                    <h3>"Run: " {&job_id}</h3>
                    <button class="modal-close" on:click=on_close_click>"✕"</button>
                </div>
                <div class="modal-body">
                    {param_values.into_iter().map(|(name, sig, required)| {
                        let label = if required {
                            format!("{} *", name)
                        } else {
                            name.clone()
                        };
                        let name_for_id = name.clone();
                        view! {
                            <div class="form-field">
                                <label for=name_for_id.clone()>{label}</label>
                                <input
                                    id=name_for_id
                                    type="text"
                                    value=sig.get_untracked()
                                    on:input=move |ev| {
                                        sig.set(event_target_value(&ev));
                                    }
                                />
                            </div>
                        }
                    }).collect_view()}
                    {move || error.get().map(|e| view! {
                        <div class="form-error">{e}</div>
                    })}
                </div>
                <div class="modal-footer">
                    <button class="btn btn-secondary" on:click=on_close_click>"Cancel"</button>
                    <button
                        class="btn btn-primary"
                        on:click=on_submit
                        disabled=move || submitting.get()
                    >
                        {move || if submitting.get() { "Running..." } else { "▶ Run" }}
                    </button>
                </div>
            </div>
        </div>
    }
}

/// DAGグラフ（SVGで描画）
#[component]
pub fn DagGraph(
    nodes: Vec<api::NodeDefinition>,
    #[prop(optional)]
    node_statuses: Option<HashMap<String, String>>,
) -> impl IntoView {
    if nodes.is_empty() {
        return view! { <div class="dag-empty">"No nodes"</div> }.into_view();
    }

    // トポロジカルレベル（各ノードのY位置）を計算
    let node_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let mut levels: HashMap<String, usize> = HashMap::new();

    // 各ノードのレベルを依存関係から計算
    fn calc_level(
        node_id: &str,
        nodes: &[api::NodeDefinition],
        levels: &mut HashMap<String, usize>,
    ) -> usize {
        if let Some(&l) = levels.get(node_id) {
            return l;
        }
        let node = nodes.iter().find(|n| n.id == node_id);
        let deps = node.map(|n| &n.depends_on).cloned().unwrap_or_default();
        let level = if deps.is_empty() {
            0
        } else {
            deps.iter()
                .map(|d| calc_level(d, nodes, levels) + 1)
                .max()
                .unwrap_or(0)
        };
        levels.insert(node_id.to_string(), level);
        level
    }

    for id in &node_ids {
        calc_level(id, &nodes, &mut levels);
    }

    // レベルごとにノードをグループ化
    let max_level = levels.values().copied().max().unwrap_or(0);
    let mut level_groups: Vec<Vec<&api::NodeDefinition>> = vec![vec![]; max_level + 1];
    for node in &nodes {
        let l = levels[&node.id];
        level_groups[l].push(node);
    }

    let node_w: f64 = 140.0;
    let node_h: f64 = 36.0;
    let h_gap: f64 = 40.0;
    let v_gap: f64 = 60.0;
    let pad: f64 = 20.0;

    // 各レベルの最大幅を求め、SVGサイズを計算
    let max_cols = level_groups.iter().map(|g| g.len()).max().unwrap_or(1);
    let svg_w = pad * 2.0 + max_cols as f64 * node_w + (max_cols as f64 - 1.0).max(0.0) * h_gap;
    let svg_h = pad * 2.0 + (max_level + 1) as f64 * node_h + max_level as f64 * v_gap;

    // ノード位置を計算
    let mut positions: HashMap<String, (f64, f64)> = HashMap::new();
    for (level, group) in level_groups.iter().enumerate() {
        let count = group.len();
        let total_w = count as f64 * node_w + (count as f64 - 1.0).max(0.0) * h_gap;
        let start_x = (svg_w - total_w) / 2.0;
        let y = pad + level as f64 * (node_h + v_gap);
        for (i, node) in group.iter().enumerate() {
            let x = start_x + i as f64 * (node_w + h_gap);
            positions.insert(node.id.clone(), (x, y));
        }
    }

    // エッジを生成
    let mut edges: Vec<(f64, f64, f64, f64)> = Vec::new();
    for node in &nodes {
        if let Some(&(tx, ty)) = positions.get(&node.id) {
            for dep in &node.depends_on {
                if let Some(&(sx, sy)) = positions.get(dep) {
                    edges.push((
                        sx + node_w / 2.0,
                        sy + node_h,
                        tx + node_w / 2.0,
                        ty,
                    ));
                }
            }
        }
    }

    let statuses = node_statuses.unwrap_or_default();

    view! {
        <div class="dag-container">
            <svg
                width=format!("{svg_w}")
                height=format!("{svg_h}")
                viewBox=format!("0 0 {svg_w} {svg_h}")
                class="dag-svg"
            >
                // marker for arrowhead
                <defs>
                    <marker id="arrow" viewBox="0 0 10 10" refX="10" refY="5"
                        markerWidth="8" markerHeight="8" orient="auto-start-reverse">
                        <path d="M 0 0 L 10 5 L 0 10 z" fill="#999" />
                    </marker>
                </defs>

                // edges
                {edges.iter().map(|&(x1, y1, x2, y2)| {
                    let mid_y = (y1 + y2) / 2.0;
                    let d = format!("M {x1} {y1} C {x1} {mid_y}, {x2} {mid_y}, {x2} {y2}");
                    view! {
                        <path d=d class="dag-edge" marker-end="url(#arrow)" />
                    }
                }).collect_view()}

                // nodes
                {nodes.iter().map(|node| {
                    let (x, y) = positions[&node.id];
                    let status = statuses.get(&node.id).cloned().unwrap_or_default();
                    let class = format!("dag-node dag-node-{}", if status.is_empty() { "default" } else { &status });
                    let label = node.name.clone();
                    // テキストを短縮（長すぎる場合）
                    let display = if label.len() > 16 {
                        format!("{}...", &label[..14])
                    } else {
                        label
                    };
                    view! {
                        <g>
                            <rect
                                x=format!("{x}")
                                y=format!("{y}")
                                width=format!("{node_w}")
                                height=format!("{node_h}")
                                rx="6"
                                class=class
                            />
                            <text
                                x=format!("{}", x + node_w / 2.0)
                                y=format!("{}", y + node_h / 2.0 + 1.0)
                                text-anchor="middle"
                                dominant-baseline="middle"
                                class="dag-label"
                            >
                                {display}
                            </text>
                        </g>
                    }
                }).collect_view()}
            </svg>
        </div>
    }.into_view()
}
