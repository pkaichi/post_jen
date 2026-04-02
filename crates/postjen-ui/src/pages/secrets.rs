use leptos::*;
use leptos_router::*;
use crate::api;

#[component]
pub fn SecretsPage() -> impl IntoView {
    let secrets = create_resource(|| (), |_| api::fetch_secrets());

    view! {
        <div class="header-row">
            <A href="/" class="back-link">"← Dashboard"</A>
        </div>

        <div class="card">
            <h2>"Secrets"</h2>
            <Suspense fallback=move || view! { <div class="loading">"Loading..."</div> }>
                {move || secrets.get().map(|result| match result {
                    Ok(secrets) => view! {
                        <table>
                            <thead>
                                <tr>
                                    <th>"Name"</th>
                                    <th>"Created"</th>
                                    <th>"Updated"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {secrets.into_iter().map(|secret| {
                                    view! {
                                        <tr>
                                            <td>{&secret.name}</td>
                                            <td>{&secret.created_at}</td>
                                            <td>{&secret.updated_at}</td>
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
