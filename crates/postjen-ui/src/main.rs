mod api;
mod components;
mod pages;

use leptos::*;
use leptos_router::*;
use pages::{dashboard::DashboardPage, run_detail::RunDetailPage, job_detail::JobDetailPage, agents::AgentsPage, secrets::SecretsPage};

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(|| view! { <App /> });
}

#[component]
fn App() -> impl IntoView {
    view! {
        <Router>
            <nav class="navbar">
                <A href="/" class="brand">"postjen"</A>
                <A href="/agents">"Agents"</A>
                <A href="/secrets">"Secrets"</A>
            </nav>
            <main class="container">
                <Routes>
                    <Route path="/" view=DashboardPage />
                    <Route path="/jobs/:job_id" view=JobDetailPage />
                    <Route path="/runs/:run_id" view=RunDetailPage />
                    <Route path="/agents" view=AgentsPage />
                    <Route path="/secrets" view=SecretsPage />
                </Routes>
            </main>
        </Router>
    }
}
