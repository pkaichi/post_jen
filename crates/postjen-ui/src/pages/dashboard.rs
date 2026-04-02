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

    view! {
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
                                                        let jid = job_id_run.clone();
                                                        spawn_local(async move {
                                                            if let Ok(resp) = api::start_run(&jid).await {
                                                                let navigate = use_navigate();
                                                                navigate(&format!("/runs/{}", resp.run_id), Default::default());
                                                            }
                                                        });
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
