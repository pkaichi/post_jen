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

    view! {
        <div class="header-row">
            <A href="/" class="back-link">"← Dashboard"</A>
            <button class="btn btn-primary" on:click=move |_| {
                let jid = job_id();
                spawn_local(async move {
                    if let Ok(resp) = api::start_run(&jid).await {
                        let navigate = use_navigate();
                        navigate(&format!("/runs/{}", resp.run_id), Default::default());
                    }
                });
            }>"▶ Run"</button>
        </div>

        <div class="card">
            <h2>{move || format!("Job: {}", job_id())}</h2>
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
