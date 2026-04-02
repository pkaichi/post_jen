use leptos::*;

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
