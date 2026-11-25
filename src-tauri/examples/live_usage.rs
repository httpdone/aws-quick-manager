//! Live validation: read the first vault account's blob and query its usage
//! (entitlements + profile/usage session bucket). Prints what the GUI shows.
use aws_quick_manager_lib::quick::{api, credstore, vault};

#[tokio::main]
async fn main() {
    let store = credstore::system();
    let entries = vault::list(&vault::dir()).expect("vault list");
    let entry = entries.first().expect("no vault accounts");
    let blob = vault::get_blob(store.as_ref(), &entry.id).expect("read").expect("no blob");
    let http = api::client();
    match api::usage_for_blob(&http, &blob).await {
        Ok((_, u)) => {
            println!("plan: {}", u.plan);
            println!("trial_ends_at: {}", u.trial_ends_at);
            println!("session_resume_minutes: {}", u.session_resume_minutes);
            println!("monthly_resets_at: {}", u.monthly_resets_at);
            for w in &u.windows {
                println!("window: {} used={} limit={} left={} unit={}", w.name, w.used_hours, w.limit_hours, w.left_hours, w.unit);
            }
        }
        Err(e) => println!("error: {e}"),
    }
}
