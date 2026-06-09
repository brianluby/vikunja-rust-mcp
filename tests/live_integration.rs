//! Optional integration test against a real Vikunja instance.
//!
//! Runs only when both `VIKUNJA_TEST_URL` and `VIKUNJA_TEST_API_TOKEN` are
//! set; otherwise the test is skipped (it passes without doing anything).
//! The token needs scopes for projects, tasks, labels and task comments.
//!
//! ```bash
//! VIKUNJA_TEST_URL=https://vikunja.example.com \
//! VIKUNJA_TEST_API_TOKEN=tk_... \
//! cargo test --test live_integration -- --nocapture
//! ```

use clap::Parser;
use vikunja_rust_mcp::config::{Cli, Config};
use vikunja_rust_mcp::vikunja::VikunjaClient;
use vikunja_rust_mcp::vikunja::client::TaskListOptions;
use vikunja_rust_mcp::vikunja::models::{LabelCreate, ProjectCreate, TaskCreate, TaskUpdate};
use vikunja_rust_mcp::vikunja::pagination::PageParams;

fn live_client() -> Option<VikunjaClient> {
    let url = std::env::var("VIKUNJA_TEST_URL").ok()?;
    let token = std::env::var("VIKUNJA_TEST_API_TOKEN").ok()?;
    if url.trim().is_empty() || token.trim().is_empty() {
        return None;
    }
    let cli = Cli::try_parse_from([
        "vikunja-rust-mcp",
        "--vikunja-url",
        &url,
        "--api-token",
        &token,
    ])
    .ok()?;
    let config = Config::from_cli(&cli).ok()?;
    VikunjaClient::new(&config).ok()
}

#[tokio::test]
async fn live_project_task_label_comment_lifecycle() {
    let Some(client) = live_client() else {
        eprintln!(
            "skipping live integration test: set VIKUNJA_TEST_URL and VIKUNJA_TEST_API_TOKEN to enable"
        );
        return;
    };

    let marker = format!(
        "mcp-it-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default()
    );

    // Project lifecycle.
    let project = client
        .create_project(&ProjectCreate {
            title: format!("{marker} project"),
            description: Some("created by vikunja-rust-mcp integration tests".into()),
            ..Default::default()
        })
        .await
        .expect("create project");

    let fetched = client.get_project(project.id).await.expect("get project");
    assert_eq!(fetched.id, project.id);

    let listed = client
        .list_projects(PageParams::default(), Some(&marker), None)
        .await
        .expect("list projects");
    assert!(listed.items.iter().any(|p| p.id == project.id));

    // Task lifecycle.
    let task = client
        .create_task(
            project.id,
            &TaskCreate {
                title: format!("{marker} task"),
                priority: Some(3),
                ..Default::default()
            },
        )
        .await
        .expect("create task");

    let updated = client
        .update_task(
            task.id,
            &TaskUpdate {
                description: Some("updated by integration test".into()),
                ..Default::default()
            },
        )
        .await
        .expect("update task");
    assert_eq!(updated.id, task.id);
    // The title must survive a partial update (merge semantics).
    assert_eq!(updated.title, task.title);

    let done = client.set_task_done(task.id, true).await.expect("complete");
    assert!(done.done);
    let reopened = client.set_task_done(task.id, false).await.expect("reopen");
    assert!(!reopened.done);

    let tasks = client
        .list_tasks(&TaskListOptions {
            project_id: Some(project.id),
            ..Default::default()
        })
        .await
        .expect("list tasks");
    assert!(tasks.items.iter().any(|t| t.id == task.id));

    // Label lifecycle.
    let label = client
        .create_label(&LabelCreate {
            title: format!("{marker} label"),
            hex_color: Some("e8b71c".into()),
            ..Default::default()
        })
        .await
        .expect("create label");

    client
        .add_task_label(task.id, label.id)
        .await
        .expect("add label to task");
    let task_labels = client
        .list_task_labels(task.id, PageParams::default(), None)
        .await
        .expect("list task labels");
    assert!(task_labels.items.iter().any(|l| l.id == label.id));
    client
        .remove_task_label(task.id, label.id)
        .await
        .expect("remove label from task");

    // Comments.
    let comment = client
        .create_task_comment(task.id, "integration test comment")
        .await
        .expect("create comment");
    let comments = client
        .list_task_comments(task.id)
        .await
        .expect("list comments");
    assert!(comments.iter().any(|c| c.id == comment.id));
    client
        .delete_task_comment(task.id, comment.id)
        .await
        .expect("delete comment");

    // Attachments.
    let upload = client
        .upload_attachment(
            task.id,
            "it-note.txt",
            b"integration test attachment".to_vec(),
        )
        .await;
    match upload {
        Ok(_) => {
            let attachments = client
                .list_task_attachments(task.id, PageParams::default())
                .await
                .expect("list attachments");
            if let Some(attachment) = attachments
                .items
                .iter()
                .find(|a| a.file.as_ref().is_some_and(|f| f.name == "it-note.txt"))
            {
                let content = client
                    .download_attachment(task.id, attachment.id)
                    .await
                    .expect("download attachment");
                assert_eq!(content.bytes, b"integration test attachment");
                client
                    .delete_attachment(task.id, attachment.id)
                    .await
                    .expect("delete attachment");
            }
        }
        Err(err) => {
            // Some instances disable attachments or the token lacks the
            // scope; do not fail the whole lifecycle test over it.
            eprintln!("attachment upload not exercised: {err}");
        }
    }

    // Cleanup (best effort, in reverse order of creation).
    if let Err(err) = client.delete_label(label.id).await {
        eprintln!("cleanup: failed to delete label {}: {err}", label.id);
    }
    if let Err(err) = client.delete_task(task.id).await {
        eprintln!("cleanup: failed to delete task {}: {err}", task.id);
    }
    if let Err(err) = client.delete_project(project.id).await {
        eprintln!("cleanup: failed to delete project {}: {err}", project.id);
    }
}
