//! Tests for [`crate::agent_profiles`] — the CRUD a client uses to see and
//! change a profile's execution knobs.
//!
//! Written because `max_retries` was added to the schema and read by the
//! runtime while being absent from every struct and statement here. A setting
//! the runtime honours but no client can see or change is the same failure as
//! a setting nothing reads — the shape `context_strategy` and
//! `allow_network_hosts` both shipped in.

use db::testing::make_test_pool;

use crate::agent_profiles::{
    create_profile, get_profile, update_profile, CreateProfileInput, UpdateProfileInput,
};

fn new_profile() -> CreateProfileInput {
    CreateProfileInput {
        name: "An agent".into(),
        description: None,
        system_prompt: None,
        provider: "anthropic".into(),
        model: "claude-opus-5".into(),
        context_strategy: None,
        persistent_memory: None,
        max_input_tokens: None,
        max_output_tokens: None,
        run_mode: None,
        cron_expression: None,
        continuous_poll_interval_secs: None,
        max_iterations: None,
        max_retries: None,
        scope: None,
    }
}

fn no_changes() -> UpdateProfileInput {
    UpdateProfileInput {
        name: None,
        description: None,
        system_prompt: None,
        provider: None,
        model: None,
        context_strategy: None,
        persistent_memory: None,
        max_input_tokens: None,
        max_output_tokens: None,
        run_mode: None,
        cron_expression: None,
        continuous_poll_interval_secs: None,
        max_iterations: None,
        max_retries: None,
    }
}

#[tokio::test]
async fn a_new_profile_gets_the_default_retry_budget() {
    let db = make_test_pool().await;

    let created = create_profile(new_profile(), &db, None).await.expect("create");

    assert_eq!(created.max_retries, 2, "three attempts in total");
}

#[tokio::test]
async fn a_retry_budget_set_at_creation_is_kept() {
    let db = make_test_pool().await;
    let input = CreateProfileInput { max_retries: Some(5), ..new_profile() };

    let created = create_profile(input, &db, None).await.expect("create");
    let loaded = get_profile(created.id.clone(), &db).await.expect("get");

    assert_eq!(loaded.max_retries, 5);
}

/// The point of the whole exercise: a client can turn the knob.
#[tokio::test]
async fn the_retry_budget_can_be_changed_through_the_profile_api() {
    let db = make_test_pool().await;
    let created = create_profile(new_profile(), &db, None).await.expect("create");

    let updated = update_profile(
        created.id.clone(),
        UpdateProfileInput { max_retries: Some(0), ..no_changes() },
        &db,
    )
    .await
    .expect("update");

    assert_eq!(updated.max_retries, 0, "retries can be switched off");
    assert_eq!(
        get_profile(created.id, &db).await.expect("get").max_retries,
        0,
        "and it survives a reload"
    );
}

/// An update that says nothing about retries must not silently reset them —
/// the same rule every other field on this form follows.
#[tokio::test]
async fn an_unrelated_update_leaves_the_retry_budget_alone() {
    let db = make_test_pool().await;
    let created = create_profile(
        CreateProfileInput { max_retries: Some(7), ..new_profile() },
        &db,
        None,
    )
    .await
    .expect("create");

    let updated = update_profile(
        created.id,
        UpdateProfileInput { name: Some("Renamed".into()), ..no_changes() },
        &db,
    )
    .await
    .expect("update");

    assert_eq!(updated.name, "Renamed");
    assert_eq!(updated.max_retries, 7);
}
