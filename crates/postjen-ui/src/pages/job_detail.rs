use leptos::*;
use leptos_router::*;
use crate::api;
use crate::components::*;

#[component]
pub fn JobDetailPage() -> impl IntoView {
    let params = use_params_map();
    let job_id = move || {
        params.with(|p| p.get("job_id").cloned().unwrap_or_default())
    };

    let runs = create_resource(move || job_id(), |jid| async move {
        api::fetch_job_runs(&jid, 20).await
    });

    let definition = create_resource(move || job_id(), |jid| async move {
        api::fetch_job_definition(&jid).await
    });

    // パラメータダイアログの状態
    let (show_dialog, set_show_dialog) = create_signal(false);

    let handle_run_click = move |_| {
        if let Some(Ok(def)) = definition.get() {
            if !def.params.is_empty() {
                set_show_dialog.set(true);
                return;
            }
        }
        let jid = job_id();
        spawn_local(async move {
            if let Ok(resp) = api::start_run(&jid, None).await {
                let navigate = use_navigate();
                navigate(&format!("/runs/{}", resp.run_id), Default::default());
            }
        });
    };

    view! {
        // パラメータダイアログ
        {move || {
            if show_dialog.get() {
                definition.get().and_then(|r| r.ok()).map(|def| {
                    let jid = job_id();
                    let p = def.params.clone();
                    view! {
                        <ParamDialog
                            job_id=jid
                            params=p
                            on_close=move |_| set_show_dialog.set(false)
                        />
                    }
                })
            } else {
                None
            }
        }}

        <div class="header-row">
            <A href="/" class="back-link">"← Dashboard"</A>
            <button class="btn btn-primary" on:click=handle_run_click>"▶ Run"</button>
        </div>

        <div class="card">
            <h2>{move || format!("Job: {}", job_id())}</h2>
            <Suspense fallback=move || view! { <Loading /> }>
                {move || definition.get().map(|result| match result {
                    Ok(def) => {
                        let desc = def.description.clone().unwrap_or_else(|| "—".to_string());
                        let triggers_display = match &def.triggers {
                            Some(t) => {
                                let mut parts = Vec::new();
                                if let Some(cron) = &t.cron {
                                    parts.push(format!("cron: {}", cron));
                                }
                                if t.webhook {
                                    parts.push("webhook".to_string());
                                }
                                if parts.is_empty() {
                                    "—".to_string()
                                } else {
                                    parts.join(", ")
                                }
                            }
                            None => "—".to_string(),
                        };
                        let params_display = if def.params.is_empty() {
                            "—".to_string()
                        } else {
                            def.params.iter().map(|p| {
                                let mut s = p.name.clone();
                                if p.required { s.push_str(" (required)"); }
                                if let Some(d) = &p.default {
                                    s.push_str(&format!(" = {d}"));
                                }
                                s
                            }).collect::<Vec<_>>().join(", ")
                        };
                        let node_count = def.nodes.len();
                        view! {
                            <div class="def-info">
                                <span class="meta-label">"Description"</span>
                                <span>{desc}</span>
                                <span class="meta-label">"Nodes"</span>
                                <span>{node_count} " nodes"</span>
                                <span class="meta-label">"Params"</span>
                                <span>{params_display}</span>
                                <span class="meta-label">"Triggers"</span>
                                <span>{triggers_display}</span>
                            </div>
                        }.into_view()
                    }
                    Err(e) => view! { <p class="form-error">{e}</p> }.into_view(),
                })}
            </Suspense>
        </div>

        // DAGグラフ
        <div class="card">
            <h2>"Node Graph"</h2>
            <Suspense fallback=move || view! { <Loading /> }>
                {move || definition.get().map(|result| match result {
                    Ok(def) => view! {
                        <DagGraph nodes=def.nodes.clone() />
                    }.into_view(),
                    Err(_) => view! {}.into_view(),
                })}
            </Suspense>
        </div>

        <div class="card">
            <h2>"Run History"</h2>
            <Suspense fallback=move || view! { <Loading /> }>
                {move || runs.get().map(|result| match result {
                    Ok(runs) => view! {
                        <table>
                            <thead>
                                <tr>
                                    <th>"ID"</th>
                                    <th>"Status"</th>
                                    <th>"Trigger"</th>
                                    <th>"Started"</th>
                                    <th>"Finished"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {runs.into_iter().map(|run| {
                                    let run_id = run.id;
                                    let status = run.status.clone();
                                    view! {
                                        <tr class="clickable" on:click=move |_| {
                                            let navigate = use_navigate();
                                            navigate(&format!("/runs/{run_id}"), Default::default());
                                        }>
                                            <td>"#" {run.id}</td>
                                            <td><StatusBadge status=status /></td>
                                            <td>{run.trigger_type}</td>
                                            <td>{run.started_at.unwrap_or_else(|| run.queued_at.unwrap_or_default())}</td>
                                            <td>{run.finished_at.unwrap_or_default()}</td>
                                        </tr>
                                    }
                                }).collect_view()}
                            </tbody>
                        </table>
                    }.into_view(),
                    Err(e) => view! { <p>"Error: " {e}</p> }.into_view(),
                })}
            </Suspense>
        </div>
    }
}
