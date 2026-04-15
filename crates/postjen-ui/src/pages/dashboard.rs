use leptos::*;
use leptos_router::*;
use crate::api;
use crate::components::*;

#[component]
pub fn DashboardPage() -> impl IntoView {
    let runs = create_resource(|| (), |_| api::fetch_runs(20));
    let jobs = create_resource(|| (), |_| api::fetch_jobs());

    // Polling: refresh every 10 seconds
    let (tick, set_tick) = create_signal(0u32);
    set_interval(move || set_tick.update(|t| *t += 1), std::time::Duration::from_secs(10));

    let runs_polling = create_resource(move || tick.get(), |_| api::fetch_runs(20));
    let jobs_polling = create_resource(move || tick.get(), |_| api::fetch_jobs());

    // パラメータダイアログの状態
    let (dialog_job_id, set_dialog_job_id) = create_signal(Option::<String>::None);
    let (dialog_params, set_dialog_params) = create_signal(Vec::<api::ParamDefinition>::new());

    let handle_run_click = move |job_id: String| {
        spawn_local(async move {
            match api::fetch_job_definition(&job_id).await {
                Ok(def) if !def.params.is_empty() => {
                    set_dialog_params.set(def.params);
                    set_dialog_job_id.set(Some(job_id));
                }
                _ => {
                    if let Ok(resp) = api::start_run(&job_id, None).await {
                        let navigate = use_navigate();
                        navigate(&format!("/runs/{}", resp.run_id), Default::default());
                    }
                }
            }
        });
    };

    view! {
        // パラメータダイアログ
        {move || dialog_job_id.get().map(|jid| {
            let params = dialog_params.get();
            view! {
                <ParamDialog
                    job_id=jid
                    params=params
                    on_close=move |_| set_dialog_job_id.set(None)
                />
            }
        })}

        <div class="card">
            <h2>"Recent Runs"</h2>
            <Suspense fallback=move || view! { <Loading /> }>
                {move || {
                    let data = runs_polling.get().or_else(|| runs.get());
                    data.map(|result| match result {
                        Ok(runs) => view! {
                            <table>
                                <thead>
                                    <tr>
                                        <th>"ID"</th>
                                        <th>"Job"</th>
                                        <th>"Status"</th>
                                        <th>"Trigger"</th>
                                        <th>"Started"</th>
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
                                                <td>{run.job_name}</td>
                                                <td><StatusBadge status=status /></td>
                                                <td>{run.trigger_type}</td>
                                                <td>{run.started_at.unwrap_or_else(|| run.queued_at.unwrap_or_default())}</td>
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
            <h2>"Jobs"</h2>
            <Suspense fallback=move || view! { <Loading /> }>
                {move || {
                    let data = jobs_polling.get().or_else(|| jobs.get());
                    let handle_run = handle_run_click.clone();
                    data.map(|result| match result {
                        Ok(jobs) => view! {
                            <table>
                                <thead>
                                    <tr>
                                        <th>"Job ID"</th>
                                        <th>"Name"</th>
                                        <th>"Enabled"</th>
                                        <th>"Action"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {jobs.into_iter().map(|job| {
                                        let job_id_run = job.job_id.clone();
                                        let job_id_nav = job.job_id.clone();
                                        let handle = handle_run.clone();
                                        view! {
                                            <tr class="clickable">
                                                <td on:click=move |_| {
                                                    let navigate = use_navigate();
                                                    navigate(&format!("/jobs/{}", job_id_nav), Default::default());
                                                }>{&job.job_id}</td>
                                                <td>{&job.name}</td>
                                                <td>{if job.enabled == 1 { "✓" } else { "—" }}</td>
                                                <td>
                                                    <button class="btn btn-primary" on:click=move |_| {
                                                        handle(job_id_run.clone());
                                                    }>"▶ Run"</button>
                                                </td>
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
    }
}
