use leptos::*;
use leptos_router::*;
use crate::api;

#[component]
pub fn AgentsPage() -> impl IntoView {
    let agents = create_resource(|| (), |_| api::fetch_agents());

    view! {
        <div class="header-row">
            <A href="/" class="back-link">"← Dashboard"</A>
        </div>

        <div class="card">
            <h2>"Agents"</h2>
            <Suspense fallback=move || view! { <div class="loading">"Loading..."</div> }>
                {move || agents.get().map(|result| match result {
                    Ok(agents) => view! {
                        <table>
                            <thead>
                                <tr>
                                    <th>"Name"</th>
                                    <th>"Hostname"</th>
                                    <th>"Labels"</th>
                                    <th>"Status"</th>
                                    <th>"Last Heartbeat"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {agents.into_iter().map(|agent| {
                                    let status_class = if agent.status == "online" { "status-success" } else { "status-canceled" };
                                    let icon = if agent.status == "online" { "●" } else { "○" };
                                    view! {
                                        <tr>
                                            <td>{&agent.name}</td>
                                            <td>{&agent.hostname}</td>
                                            <td>{&agent.labels_json}</td>
                                            <td><span class=format!("status {status_class}")>{icon} " " {&agent.status}</span></td>
                                            <td>{&agent.last_heartbeat_at}</td>
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
