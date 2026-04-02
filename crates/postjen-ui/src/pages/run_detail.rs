use leptos::*;
use leptos_router::*;
use crate::api;
use crate::components::*;

#[component]
pub fn RunDetailPage() -> impl IntoView {
    let params = use_params_map();
    let run_id = move || {
        params.with(|p| p.get("run_id").and_then(|s| s.parse::<i64>().ok()).unwrap_or(0))
    };

    let run = create_resource(move || run_id(), api::fetch_run);
    let nodes = create_resource(move || run_id(), api::fetch_run_nodes);
    let logs = create_resource(move || run_id(), |rid| async move {
        api::fetch_run_logs(rid, None).await
    });

    // Polling for live updates
    let (tick, set_tick) = create_signal(0u32);
    set_interval(move || set_tick.update(|t| *t += 1), std::time::Duration::from_secs(3));

    let run_live = create_resource(
        move || (run_id(), tick.get()),
        |(rid, _)| api::fetch_run(rid),
    );
    let nodes_live = create_resource(
        move || (run_id(), tick.get()),
        |(rid, _)| api::fetch_run_nodes(rid),
    );
    let logs_live = create_resource(
        move || (run_id(), tick.get()),
        |(rid, _)| async move { api::fetch_run_logs(rid, None).await },
    );

    view! {
        <div class="header-row">
            <A href="/" class="back-link">"← Dashboard"</A>
            <div style="display:flex;gap:8px;">
                <button class="btn btn-danger" on:click=move |_| {
                    let rid = run_id();
                    spawn_local(async move { let _ = api::cancel_run(rid).await; });
                }>"Cancel"</button>
                <button class="btn btn-secondary" on:click=move |_| {
                    let rid = run_id();
                    spawn_local(async move {
                        if let Ok(resp) = api::rerun_run(rid).await {
                            let navigate = use_navigate();
                            navigate(&format!("/runs/{}", resp.run_id), Default::default());
                        }
                    });
                }>"Rerun"</button>
            </div>
        </div>

        <div class="card">
            <Suspense fallback=move || view! { <Loading /> }>
                {move || {
                    let data = run_live.get().or_else(|| run.get());
                    data.map(|result| match result {
                        Ok(r) => {
                            let status = r.status.clone();
                            let trigger = r.trigger_type.clone();
                            let triggered_by = r.triggered_by.clone().unwrap_or_default();
                            let params_display = r.params_json.clone()
                                .filter(|s| !s.is_empty() && s != "null")
                                .unwrap_or_else(|| "—".to_string());
                            let started = r.started_at.clone().unwrap_or_else(|| "—".to_string());
                            let finished = r.finished_at.clone().unwrap_or_else(|| "—".to_string());
                            let failure = r.failure_reason.clone();
                            let job_name = r.job_name.clone();
                            let id = r.id;
                            view! {
                                <div class="header-row">
                                    <h2>"Run #" {id} " — " {job_name}</h2>
                                    <StatusBadge status=status />
                                </div>
                                <div class="meta-grid">
                                    <span class="meta-label">"Trigger"</span>
                                    <span>{trigger} " " {triggered_by}</span>
                                    <span class="meta-label">"Params"</span>
                                    <span>{params_display}</span>
                                    <span class="meta-label">"Started"</span>
                                    <span>{started}</span>
                                    <span class="meta-label">"Finished"</span>
                                    <span>{finished}</span>
                                    {failure.map(|reason| view! {
                                        <span class="meta-label">"Failure"</span>
                                        <span style="color:#ef4444">{reason}</span>
                                    })}
                                </div>
                            }.into_view()
                        }
                        Err(e) => view! { <p>"Error: " {e}</p> }.into_view(),
                    })
                }}
            </Suspense>
        </div>

        <div class="card">
            <h2>"Nodes"</h2>
            <Suspense fallback=move || view! { <Loading /> }>
                {move || {
                    let data = nodes_live.get().or_else(|| nodes.get());
                    data.map(|result| match result {
                        Ok(node_runs) => view! {
                            <table>
                                <thead>
                                    <tr>
                                        <th>"Node"</th>
                                        <th>"Status"</th>
                                        <th>"Agent"</th>
                                        <th>"Started"</th>
                                        <th>"Finished"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {node_runs.into_iter().map(|nr| {
                                        let status = nr.status.clone();
                                        let name = nr.node_name.clone().unwrap_or_else(|| nr.node_id.clone());
                                        let agent = nr.assigned_agent_id.clone().unwrap_or_else(|| "—".to_string());
                                        let started = nr.started_at.clone().unwrap_or_else(|| "—".to_string());
                                        let finished = nr.finished_at.clone().unwrap_or_else(|| "—".to_string());
                                        view! {
                                            <tr>
                                                <td>{name}</td>
                                                <td><StatusBadge status=status /></td>
                                                <td>{agent}</td>
                                                <td>{started}</td>
                                                <td>{finished}</td>
                                            </tr>
                                        }
                                    }).collect_view()}
                                </tbody>
                            </table>
                        }.into_view(),
                        Err(e) => view! { <p>"Error: " {e}</p> }.into_view(),
                    })
                }}
            </Suspense>
        </div>

        <div class="card">
            <h2>"Logs"</h2>
            <Suspense fallback=move || view! { <Loading /> }>
                {move || {
                    let data = logs_live.get().or_else(|| logs.get());
                    data.map(|result| match result {
                        Ok(entries) => view! {
                            <div class="log-viewer">
                                {entries.into_iter().map(|entry| {
                                    let style = match entry.stream.as_str() {
                                        "stderr" => "color: #f87171;",
                                        "system" => "color: #60a5fa; font-style: italic;",
                                        _ => "",
                                    };
                                    let content = entry.content.clone();
                                    view! {
                                        <div style=style>{content}</div>
                                    }
                                }).collect_view()}
                            </div>
                        }.into_view(),
                        Err(e) => view! { <p>"Error: " {e}</p> }.into_view(),
                    })
                }}
            </Suspense>
        </div>
    }
}
