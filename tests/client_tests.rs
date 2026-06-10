//! Mocked-HTTP tests for the Vikunja API client: request building, auth,
//! pagination, merge-update semantics, error mapping and retries.

mod common;

use std::time::Duration;

use common::{TEST_TOKEN, test_client, test_client_with_timeout};
use serde_json::json;
use vikunja_rust_mcp::error::{ApiErrorKind, Error};
use vikunja_rust_mcp::vikunja::client::{TaskListOptions, saved_filter_options};
use vikunja_rust_mcp::vikunja::models::{
    LabelCreate, LabelUpdate, ProjectCreate, ProjectUpdate, RelationKind, SavedFilter,
    SavedFilterCreate, SavedFilterQuery, SavedFilterUpdate, TaskCreate, TaskReminder, TaskUpdate,
};
use vikunja_rust_mcp::vikunja::pagination::PageParams;
use wiremock::matchers::{body_json, body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn project_json(id: i64, title: &str) -> serde_json::Value {
    json!({
        "id": id, "title": title, "description": "", "identifier": "",
        "hex_color": "", "parent_project_id": 0, "is_archived": false,
        "is_favorite": false, "position": 0,
        "owner": {"id": 1, "username": "ada"},
        "created": "2026-01-01T00:00:00Z", "updated": "2026-01-01T00:00:00Z"
    })
}

fn task_json(id: i64, title: &str, done: bool) -> serde_json::Value {
    json!({
        "id": id, "title": title, "description": "", "done": done,
        "project_id": 3, "priority": 0, "percent_done": 0.0,
        "identifier": format!("TEST-{id}"), "index": id, "hex_color": "",
        "is_favorite": false, "repeat_after": 0,
        "labels": null, "assignees": null
    })
}

// ----- auth & request building ------------------------------------------------

#[tokio::test]
async fn sends_bearer_auth_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects"))
        .and(header("authorization", format!("Bearer {TEST_TOKEN}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let page = client
        .list_projects(PageParams::default(), None, None)
        .await
        .unwrap();
    assert!(page.items.is_empty());
}

#[tokio::test]
async fn list_projects_sends_pagination_and_search_params() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects"))
        .and(query_param("page", "2"))
        .and(query_param("per_page", "10"))
        .and(query_param("s", "inbox"))
        .and(query_param("is_archived", "true"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([project_json(1, "Inbox")]))
                .insert_header("x-pagination-total-pages", "4")
                .insert_header("x-pagination-result-count", "1"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let page = client
        .list_projects(
            PageParams::new(Some(2), Some(10)),
            Some("inbox"),
            Some(true),
        )
        .await
        .unwrap();

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].title, "Inbox");
    assert_eq!(page.info.page, 2);
    assert_eq!(page.info.per_page, Some(10));
    assert_eq!(page.info.total_pages, Some(4));
    assert_eq!(page.info.result_count, Some(1));
    assert_eq!(page.info.has_more, Some(true));
}

#[tokio::test]
async fn list_projects_applies_default_per_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects"))
        .and(query_param("per_page", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    client
        .list_projects(PageParams::default(), None, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn null_body_yields_empty_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/labels"))
        .respond_with(ResponseTemplate::new(200).set_body_string("null"))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let page = client
        .list_labels(PageParams::default(), None)
        .await
        .unwrap();
    assert!(page.items.is_empty());
}

// ----- projects ----------------------------------------------------------------

#[tokio::test]
async fn get_project_hits_id_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(project_json(7, "Work")))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let project = client.get_project(7).await.unwrap();
    assert_eq!(project.id, 7);
    assert_eq!(project.title, "Work");
    assert_eq!(project.owner.unwrap().username, "ada");
}

#[tokio::test]
async fn create_project_uses_put_with_body() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/projects"))
        .and(body_json(json!({"title": "New", "hex_color": "00ff00"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(project_json(9, "New")))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let project = client
        .create_project(&ProjectCreate {
            title: "New".into(),
            hex_color: Some("00ff00".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(project.id, 9);
}

#[tokio::test]
async fn update_project_merges_current_state() {
    let server = MockServer::start().await;
    // Current project state, fetched before the write.
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 7, "title": "Old title", "description": "keep me",
            "hex_color": "112233", "is_archived": false, "is_favorite": true
        })))
        .expect(1)
        .mount(&server)
        .await;
    // The write must contain the merged object: new title, kept description.
    Mock::given(method("POST"))
        .and(path("/api/v1/projects/7"))
        .and(body_json(json!({
            "id": 7, "title": "New title", "description": "keep me",
            "hex_color": "112233", "is_archived": false, "is_favorite": true
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(project_json(7, "New title")))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let project = client
        .update_project(
            7,
            &ProjectUpdate {
                title: Some("New title".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(project.title, "New title");
}

#[tokio::test]
async fn update_project_rejects_empty_patch() {
    // No mocks: the call must fail before any request is made.
    let client = test_client("http://127.0.0.1:1");
    let err = client
        .update_project(7, &ProjectUpdate::default())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::InvalidArgument(_)), "got {err:?}");
}

#[tokio::test]
async fn delete_project_returns_message() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/projects/7"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"message": "Successfully deleted."})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let message = client.delete_project(7).await.unwrap();
    assert_eq!(message.message, "Successfully deleted.");
}

// ----- tasks --------------------------------------------------------------------

#[tokio::test]
async fn list_tasks_combines_filter_and_project() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks"))
        .and(query_param("filter", "(done = false) && project_id = 5"))
        .and(query_param("s", "report"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!([task_json(1, "Report", false)])),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let page = client
        .list_tasks(&TaskListOptions {
            search: Some("report".into()),
            filter: Some("done = false".into()),
            project_id: Some(5),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
}

#[tokio::test]
async fn list_tasks_passes_sort_params() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks"))
        .and(query_param("sort_by", "due_date"))
        .and(query_param("order_by", "desc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    client
        .list_tasks(&TaskListOptions {
            sort_by: Some("due_date".into()),
            order_by: Some("desc".into()),
            ..Default::default()
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn get_task_parses_full_shape() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 42, "title": "Ship it", "description": "soon", "done": false,
            "due_date": "2026-07-01T12:00:00Z", "priority": 3, "project_id": 5,
            "labels": [{"id": 1, "title": "urgent", "hex_color": "ff0000", "description": ""}],
            "assignees": [{"id": 2, "username": "ada"}]
        })))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let task = client.get_task(42).await.unwrap();
    assert_eq!(task.title, "Ship it");
    assert_eq!(task.due_date.as_deref(), Some("2026-07-01T12:00:00Z"));
    assert_eq!(task.labels.unwrap()[0].title, "urgent");
    assert_eq!(task.assignees.unwrap()[0].id, 2);
}

#[tokio::test]
async fn create_task_uses_project_scoped_put() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/projects/3/tasks"))
        .and(body_json(json!({
            "title": "New task", "due_date": "2026-07-01T12:00:00Z", "priority": 2
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(task_json(99, "New task", false)))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let task = client
        .create_task(
            3,
            &TaskCreate {
                title: "New task".into(),
                due_date: Some("2026-07-01T12:00:00Z".into()),
                priority: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(task.id, 99);
}

#[tokio::test]
async fn update_task_merges_current_state() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "title": "Keep title", "description": "keep description",
            "done": false, "priority": 1, "project_id": 3
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/9"))
        .and(body_json(json!({
            "id": 9, "title": "Keep title", "description": "keep description",
            "done": false, "priority": 4, "project_id": 3
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(task_json(9, "Keep title", false)))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let task = client
        .update_task(
            9,
            &TaskUpdate {
                priority: Some(4),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(task.id, 9);
}

#[tokio::test]
async fn set_task_done_posts_merged_done_flag() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "title": "Task", "done": false, "project_id": 3
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/9"))
        .and(body_json(json!({
            "id": 9, "title": "Task", "done": true, "project_id": 3
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(task_json(9, "Task", true)))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let task = client.set_task_done(9, true).await.unwrap();
    assert!(task.done);
}

#[tokio::test]
async fn set_task_done_false_reopens() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "title": "Task", "done": true, "project_id": 3
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/9"))
        .and(body_json(json!({
            "id": 9, "title": "Task", "done": false, "project_id": 3
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(task_json(9, "Task", false)))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let task = client.set_task_done(9, false).await.unwrap();
    assert!(!task.done);
}

#[tokio::test]
async fn delete_task_works() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tasks/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "ok"})))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    assert_eq!(client.delete_task(9).await.unwrap().message, "ok");
}

// ----- labels --------------------------------------------------------------------

#[tokio::test]
async fn labels_list_get_create_update_delete() {
    let server = MockServer::start().await;
    let label = json!({"id": 5, "title": "bug", "description": "", "hex_color": "ff0000"});

    Mock::given(method("GET"))
        .and(path("/api/v1/labels"))
        .and(query_param("s", "bu"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([label])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/labels/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(label.clone()))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/labels"))
        .and(body_json(json!({"title": "bug", "hex_color": "ff0000"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(label.clone()))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/labels/5"))
        .and(body_json(json!({
            "id": 5, "title": "defect", "description": "", "hex_color": "ff0000"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 5, "title": "defect", "description": "", "hex_color": "ff0000"
        })))
        .mount(&server)
        .await;
    // Vikunja answers label deletion with the deleted label, not a message.
    Mock::given(method("DELETE"))
        .and(path("/api/v1/labels/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(label.clone()))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());

    let page = client
        .list_labels(PageParams::default(), Some("bu"))
        .await
        .unwrap();
    assert_eq!(page.items[0].title, "bug");

    assert_eq!(client.get_label(5).await.unwrap().id, 5);

    let created = client
        .create_label(&LabelCreate {
            title: "bug".into(),
            hex_color: Some("ff0000".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(created.id, 5);

    let updated = client
        .update_label(
            5,
            &LabelUpdate {
                title: Some("defect".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.title, "defect");

    let deleted = client.delete_label(5).await.unwrap();
    assert_eq!(deleted.id, 5);
    assert_eq!(deleted.title, "bug");
}

// ----- task labels -----------------------------------------------------------------

#[tokio::test]
async fn task_labels_list_add_remove() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9/labels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 2, "title": "urgent", "description": "", "hex_color": ""}
        ])))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/tasks/9/labels"))
        .and(body_json(json!({"label_id": 2})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"label_id": 2})))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tasks/9/labels/2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "removed"})))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());

    let labels = client
        .list_task_labels(9, PageParams::default(), None)
        .await
        .unwrap();
    assert_eq!(labels.items[0].id, 2);

    let added = client.add_task_label(9, 2).await.unwrap();
    assert_eq!(added.label_id, 2);

    let removed = client.remove_task_label(9, 2).await.unwrap();
    assert_eq!(removed.message, "removed");
}

// ----- assignees --------------------------------------------------------------------

#[tokio::test]
async fn assignees_list_add_remove() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9/assignees"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 3, "username": "grace"}
        ])))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/tasks/9/assignees"))
        .and(body_json(json!({"user_id": 3})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"user_id": 3})))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tasks/9/assignees/3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "unassigned"})))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());

    let assignees = client
        .list_task_assignees(9, PageParams::default())
        .await
        .unwrap();
    assert_eq!(assignees.items[0].username, "grace");

    let assigned = client.assign_user(9, 3).await.unwrap();
    assert_eq!(assigned.user_id, 3);

    let removed = client.unassign_user(9, 3).await.unwrap();
    assert_eq!(removed.message, "unassigned");
}

// ----- comments ----------------------------------------------------------------------

#[tokio::test]
async fn comments_list_create_update_delete() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "comment": "hello", "author": {"id": 1, "username": "ada"}}
        ])))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/tasks/9/comments"))
        .and(body_json(json!({"comment": "new comment"})))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(json!({"id": 2, "comment": "new comment"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/9/comments/2"))
        .and(body_json(json!({"comment": "edited"})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id": 2, "comment": "edited"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tasks/9/comments/2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "deleted"})))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());

    let comments = client.list_task_comments(9).await.unwrap();
    assert_eq!(comments[0].comment, "hello");

    let created = client.create_task_comment(9, "new comment").await.unwrap();
    assert_eq!(created.id, 2);

    let updated = client.update_task_comment(9, 2, "edited").await.unwrap();
    assert_eq!(updated.comment, "edited");

    let deleted = client.delete_task_comment(9, 2).await.unwrap();
    assert_eq!(deleted.message, "deleted");
}

#[tokio::test]
async fn comments_null_body_is_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_string("null"))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    assert!(client.list_task_comments(9).await.unwrap().is_empty());
}

// ----- attachments --------------------------------------------------------------------

#[tokio::test]
async fn attachments_list_parses_file_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "id": 4, "task_id": 9,
            "file": {"id": 11, "name": "notes.txt", "mime": "text/plain", "size": 12},
            "created_by": {"id": 1, "username": "ada"}
        }])))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let page = client
        .list_task_attachments(9, PageParams::default())
        .await
        .unwrap();
    let file = page.items[0].file.as_ref().unwrap();
    assert_eq!(file.name, "notes.txt");
    assert_eq!(file.size, Some(12));
}

#[tokio::test]
async fn attachment_upload_sends_multipart_file() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/tasks/9/attachments"))
        .and(body_string_contains("name=\"files\""))
        .and(body_string_contains("filename=\"notes.txt\""))
        .and(body_string_contains("hello attachment"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"message": "Attachments were uploaded successfully."})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let message = client
        .upload_attachment(9, "notes.txt", b"hello attachment".to_vec())
        .await
        .unwrap();
    assert!(message.message.contains("uploaded"));
}

#[tokio::test]
async fn attachment_download_returns_bytes_and_mime() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9/attachments/4"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(b"file-content".to_vec())
                .insert_header("content-type", "text/plain"),
        )
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let content = client.download_attachment(9, 4, 1024).await.unwrap();
    assert_eq!(content.bytes, b"file-content");
    assert_eq!(content.content_type.as_deref(), Some("text/plain"));
}

#[tokio::test]
async fn attachment_download_rejects_bodies_over_the_cap() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9/attachments/4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 64]))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let err = client.download_attachment(9, 4, 16).await.unwrap_err();
    match err {
        Error::TooLarge { size, limit, .. } => {
            // wiremock sends Content-Length, so the request is rejected
            // before the body is read.
            assert_eq!(size, Some(64));
            assert_eq!(limit, 16);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn attachment_download_to_file_streams_to_disk() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9/attachments/4"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(b"streamed to disk".to_vec())
                .insert_header("content-type", "application/octet-stream"),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("attachment.bin");

    let client = test_client(&server.uri());
    let (written, mime) = client
        .download_attachment_to_file(9, 4, target.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(written, 16);
    assert_eq!(mime.as_deref(), Some("application/octet-stream"));
    assert_eq!(std::fs::read(&target).unwrap(), b"streamed to disk");
}

#[tokio::test]
async fn attachment_download_to_unwritable_path_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9/attachments/4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"x".to_vec()))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let err = client
        .download_attachment_to_file(9, 4, "/nonexistent-dir/file.bin")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Io { .. }), "got {err:?}");
}

#[tokio::test]
async fn attachment_delete_works() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tasks/9/attachments/4"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "gone"})))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    assert_eq!(
        client.delete_attachment(9, 4).await.unwrap().message,
        "gone"
    );
}

// ----- users & teams -------------------------------------------------------------------

#[tokio::test]
async fn users_search_sends_query() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users"))
        .and(query_param("s", "ada"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "username": "ada", "name": "Ada Lovelace"}
        ])))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let users = client.search_users(Some("ada")).await.unwrap();
    assert_eq!(users[0].username, "ada");
}

#[tokio::test]
async fn teams_list_and_project_teams() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/teams"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "name": "devs", "description": ""}
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/7/teams"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "name": "devs", "description": "", "permission": 2,
             "members": [{"id": 1, "username": "ada", "admin": true}]}
        ])))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());

    let teams = client
        .list_teams(PageParams::default(), None)
        .await
        .unwrap();
    assert_eq!(teams.items[0].name, "devs");
    assert_eq!(teams.items[0].permission, None);

    let project_teams = client
        .list_project_teams(7, PageParams::default(), None)
        .await
        .unwrap();
    assert_eq!(project_teams.items[0].permission, Some(2));
    assert!(project_teams.items[0].members.as_ref().unwrap()[0].admin);
}

// ----- error mapping ---------------------------------------------------------------------

async fn error_for_status(status: u16, body: serde_json::Value) -> Error {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/1"))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(&server)
        .await;
    let client = test_client(&server.uri());
    client.get_project(1).await.unwrap_err()
}

#[tokio::test]
async fn http_401_maps_to_auth_error() {
    let err = error_for_status(401, json!({"code": 1001, "message": "invalid token"})).await;
    match err {
        Error::Api {
            kind,
            status,
            code,
            message,
            ..
        } => {
            assert_eq!(kind, ApiErrorKind::Auth);
            assert_eq!(status, 401);
            assert_eq!(code, Some(1001));
            assert_eq!(message, "invalid token");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn http_404_maps_to_not_found() {
    let err = error_for_status(404, json!({"code": 3001, "message": "project not found"})).await;
    match &err {
        Error::Api { kind, .. } => assert_eq!(*kind, ApiErrorKind::NotFound),
        other => panic!("unexpected: {other:?}"),
    }
    // And it converts to an MCP invalid_params error.
    let mcp = err.to_mcp();
    assert_eq!(mcp.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}

#[tokio::test]
async fn http_422_maps_to_validation() {
    let err = error_for_status(422, json!({"code": 0, "message": "title required"})).await;
    match err {
        Error::Api { kind, .. } => assert_eq!(kind, ApiErrorKind::Validation),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn http_429_maps_to_rate_limited() {
    let err = error_for_status(429, json!({"message": "slow down"})).await;
    match err {
        Error::Api { kind, .. } => assert_eq!(kind, ApiErrorKind::RateLimited),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn http_500_with_non_json_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/1"))
        .respond_with(ResponseTemplate::new(500).set_body_string("<html>boom</html>"))
        .mount(&server)
        .await;
    let client = test_client(&server.uri());
    let err = client.get_project(1).await.unwrap_err();
    match err {
        Error::Api { kind, message, .. } => {
            assert_eq!(kind, ApiErrorKind::Server);
            assert!(message.contains("boom"));
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn invalid_json_success_body_is_invalid_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/1"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json at all"))
        .mount(&server)
        .await;
    let client = test_client(&server.uri());
    let err = client.get_project(1).await.unwrap_err();
    assert!(matches!(err, Error::InvalidResponse { .. }), "got {err:?}");
}

#[tokio::test]
async fn connection_refused_maps_to_network_error() {
    // Port 1 is never listening.
    let client = test_client("http://127.0.0.1:1");
    let err = client.get_project(1).await.unwrap_err();
    match err {
        Error::Network { detail, .. } => assert!(detail.contains("connection failed")),
        other => panic!("unexpected: {other:?}"),
    }
}

// ----- timeouts & retry --------------------------------------------------------------------

#[tokio::test]
async fn get_retries_once_after_timeout() {
    let server = MockServer::start().await;
    // First request: slower than the 1s client timeout. Consumed once.
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(1500))
                .set_body_json(project_json(1, "slow")),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    // Second request: fast.
    Mock::given(method("GET"))
        .and(path("/api/v1/projects/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(project_json(1, "fast")))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client_with_timeout(&server.uri(), 1);
    let project = client.get_project(1).await.unwrap();
    assert_eq!(project.title, "fast");
}

#[tokio::test]
async fn writes_are_not_retried_on_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/projects"))
        .respond_with(
            ResponseTemplate::new(201)
                .set_delay(Duration::from_millis(1500))
                .set_body_json(project_json(1, "slow")),
        )
        .expect(1) // exactly one attempt: no retry for non-idempotent calls
        .mount(&server)
        .await;

    let client = test_client_with_timeout(&server.uri(), 1);
    let err = client
        .create_project(&ProjectCreate {
            title: "slow".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Timeout { .. }), "got {err:?}");
}

// ----- bulk pagination ------------------------------------------------------------------------

#[tokio::test]
async fn list_all_projects_walks_every_page() {
    let server = MockServer::start().await;
    for page in 1..=3u32 {
        Mock::given(method("GET"))
            .and(path("/api/v1/projects"))
            .and(query_param("page", page.to_string()))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([project_json(page as i64, &format!("P{page}"))]))
                    .insert_header("x-pagination-total-pages", "3"),
            )
            .expect(1)
            .mount(&server)
            .await;
    }

    let client = test_client(&server.uri());
    let projects = client.list_all_projects(10).await.unwrap();
    assert_eq!(projects.len(), 3);
    assert_eq!(projects[2].title, "P3");
}

#[tokio::test]
async fn list_all_tasks_respects_page_cap() {
    let server = MockServer::start().await;
    for page in 1..=2u32 {
        Mock::given(method("GET"))
            .and(path("/api/v1/tasks"))
            .and(query_param("page", page.to_string()))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([task_json(page as i64, "t", false)]))
                    .insert_header("x-pagination-total-pages", "100"),
            )
            .expect(1)
            .mount(&server)
            .await;
    }

    let client = test_client(&server.uri());
    // Cap at 2 pages even though the server reports 100.
    let tasks = client.list_all_tasks(2).await.unwrap();
    assert_eq!(tasks.len(), 2);
}

#[tokio::test]
async fn list_all_tasks_with_options_forwards_query_and_reports_metadata() {
    let server = MockServer::start().await;
    for page in 1..=2u32 {
        Mock::given(method("GET"))
            .and(path("/api/v1/tasks"))
            .and(query_param("page", page.to_string()))
            .and(query_param("filter", "done = false && priority >= 3"))
            .and(query_param("sort_by", "priority"))
            .and(query_param("order_by", "desc"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([task_json(page as i64, "t", false)]))
                    .insert_header("x-pagination-total-pages", "2"),
            )
            .expect(1)
            .mount(&server)
            .await;
    }

    let client = test_client(&server.uri());
    let options = TaskListOptions {
        filter: Some("done = false && priority >= 3".into()),
        sort_by: Some("priority".into()),
        order_by: Some("desc".into()),
        ..Default::default()
    };
    let result = client
        .list_all_tasks_with_options(&options, 10)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 2);
    assert_eq!(result.pages_read, 2);
    assert_eq!(result.page_cap, 10);
    assert!(!result.truncated);
    assert_eq!(result.last_info.page, 2);
    assert_eq!(result.last_info.total_pages, Some(2));
}

#[tokio::test]
async fn list_all_tasks_with_options_reports_truncation_at_cap() {
    let server = MockServer::start().await;
    for page in 1..=2u32 {
        Mock::given(method("GET"))
            .and(path("/api/v1/tasks"))
            .and(query_param("page", page.to_string()))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([task_json(page as i64, "t", false)]))
                    .insert_header("x-pagination-total-pages", "100"),
            )
            .expect(1)
            .mount(&server)
            .await;
    }

    let client = test_client(&server.uri());
    let result = client
        .list_all_tasks_with_options(&TaskListOptions::default(), 2)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 2);
    assert_eq!(result.pages_read, 2);
    assert_eq!(result.page_cap, 2);
    assert!(result.truncated, "cap hit with more pages must truncate");
}

#[tokio::test]
async fn list_all_tasks_with_options_handles_null_page_bodies() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks"))
        .and(query_param("page", "1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::Value::Null)
                .insert_header("x-pagination-total-pages", "2"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks"))
        .and(query_param("page", "2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([task_json(1, "t", false)]))
                .insert_header("x-pagination-total-pages", "2"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let result = client
        .list_all_tasks_with_options(&TaskListOptions::default(), 10)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.pages_read, 2);
    assert!(!result.truncated);
}

#[tokio::test]
async fn probe_reports_success_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    assert_eq!(client.probe().await.unwrap().as_u16(), 200);
}

// ----- saved filters ----------------------------------------------------------

fn saved_filter_json(id: i64, title: &str, filter: &str) -> serde_json::Value {
    json!({
        "id": id, "title": title, "description": "open work",
        "filters": {
            "sort_by": ["due_date", "id"],
            "order_by": ["asc", "desc"],
            "filter": filter,
            "filter_timezone": "America/Los_Angeles",
            "filter_include_nulls": false
        },
        "owner": {"id": 1, "username": "ada"},
        "is_favorite": true,
        "created": "2026-01-01T00:00:00Z", "updated": "2026-01-02T00:00:00Z"
    })
}

#[tokio::test]
async fn get_saved_filter_decodes_definition() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/filters/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(saved_filter_json(
            9,
            "Open work",
            "done = false",
        )))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let filter = client.get_saved_filter(9).await.unwrap();
    assert_eq!(filter.id, 9);
    assert_eq!(filter.title, "Open work");
    assert!(filter.is_favorite);
    let query = filter.filters.unwrap();
    assert_eq!(query.filter.as_deref(), Some("done = false"));
    assert_eq!(
        query.sort_by.as_deref(),
        Some(["due_date".to_string(), "id".to_string()].as_slice())
    );
    assert_eq!(
        query.filter_timezone.as_deref(),
        Some("America/Los_Angeles")
    );
    assert_eq!(query.filter_include_nulls, Some(false));
}

#[tokio::test]
async fn create_saved_filter_puts_only_set_fields() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/filters"))
        .and(body_json(json!({
            "title": "Open work",
            "filters": {"filter": "done = false"}
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(saved_filter_json(
            9,
            "Open work",
            "done = false",
        )))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let body = SavedFilterCreate {
        title: "Open work".into(),
        filters: SavedFilterQuery {
            filter: Some("done = false".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let filter = client.create_saved_filter(&body).await.unwrap();
    assert_eq!(filter.id, 9);
}

#[tokio::test]
async fn update_saved_filter_merges_top_level_and_nested_query() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/filters/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(saved_filter_json(
            9,
            "Open work",
            "done = false",
        )))
        .expect(1)
        .mount(&server)
        .await;
    // The merged write must keep the stored title, sort order and timezone
    // while replacing only the filter expression.
    Mock::given(method("POST"))
        .and(path("/api/v1/filters/9"))
        .and(body_string_contains("\"title\":\"Open work\""))
        .and(body_string_contains("\"filter\":\"priority >= 3\""))
        .and(body_string_contains(
            "\"filter_timezone\":\"America/Los_Angeles\"",
        ))
        .and(body_string_contains("\"due_date\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(saved_filter_json(
            9,
            "Open work",
            "priority >= 3",
        )))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let patch = SavedFilterUpdate {
        filters: Some(SavedFilterQuery {
            filter: Some("priority >= 3".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let updated = client.update_saved_filter(9, &patch).await.unwrap();
    let query = updated.filters.unwrap();
    assert_eq!(query.filter.as_deref(), Some("priority >= 3"));
}

#[tokio::test]
async fn update_saved_filter_rejects_empty_patch() {
    let client = test_client("http://127.0.0.1:1");
    let err = client
        .update_saved_filter(9, &SavedFilterUpdate::default())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::InvalidArgument(_)));
}

#[tokio::test]
async fn delete_saved_filter_returns_message() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/filters/9"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"message": "Successfully deleted."})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let message = client.delete_saved_filter(9).await.unwrap();
    assert_eq!(message.message, "Successfully deleted.");
}

#[tokio::test]
async fn list_saved_filters_derives_from_pseudo_projects() {
    let server = MockServer::start().await;
    // Vikunja reports saved filters inside the project list as
    // pseudo-projects with ids <= -2 (filter id = -project_id - 1).
    Mock::given(method("GET"))
        .and(path("/api/v1/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            project_json(1, "Inbox"),
            project_json(-2, "Open work"),
            project_json(-3, "This week"),
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let filters = client.list_saved_filters(10).await.unwrap();
    assert_eq!(filters.len(), 2);
    assert_eq!(filters[0].filter_id, 1);
    assert_eq!(filters[0].pseudo_project_id, -2);
    assert_eq!(filters[0].title, "Open work");
    assert_eq!(filters[1].filter_id, 2);
    assert_eq!(filters[1].pseudo_project_id, -3);
}

#[tokio::test]
async fn saved_filter_options_maps_stored_query() {
    let filter: SavedFilter =
        serde_json::from_value(saved_filter_json(9, "Open work", "done = false")).unwrap();
    let options = saved_filter_options(&filter);
    assert_eq!(options.filter.as_deref(), Some("done = false"));
    // Only the first sort pair maps onto /tasks, which takes one of each.
    assert_eq!(options.sort_by.as_deref(), Some("due_date"));
    assert_eq!(options.order_by.as_deref(), Some("asc"));
    assert_eq!(
        options.filter_timezone.as_deref(),
        Some("America/Los_Angeles")
    );
    assert_eq!(options.filter_include_nulls, Some(false));
}

#[tokio::test]
async fn list_tasks_sends_timezone_and_include_nulls() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks"))
        .and(query_param("filter", "done = false"))
        .and(query_param("filter_timezone", "America/Los_Angeles"))
        .and(query_param("filter_include_nulls", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let options = TaskListOptions {
        filter: Some("done = false".into()),
        filter_timezone: Some("America/Los_Angeles".into()),
        filter_include_nulls: Some(true),
        ..Default::default()
    };
    client.list_tasks(&options).await.unwrap();
}

// ----- task relations ---------------------------------------------------------

#[tokio::test]
async fn create_task_relation_puts_kind_and_ids() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/tasks/5/relations"))
        .and(body_json(json!({
            "task_id": 5, "other_task_id": 9, "relation_kind": "blocking"
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "task_id": 5, "other_task_id": 9, "relation_kind": "blocking",
            "created_by": {"id": 1, "username": "ada"},
            "created": "2026-01-01T00:00:00Z"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let relation = client
        .create_task_relation(5, 9, RelationKind::Blocking)
        .await
        .unwrap();
    assert_eq!(relation.task_id, 5);
    assert_eq!(relation.other_task_id, 9);
    assert_eq!(relation.relation_kind, RelationKind::Blocking);
    assert_eq!(relation.created_by.unwrap().username, "ada");
}

#[tokio::test]
async fn delete_task_relation_encodes_kind_in_path() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tasks/5/relations/subtask/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": "The task relation was successfully deleted."
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let message = client
        .delete_task_relation(5, 9, RelationKind::Subtask)
        .await
        .unwrap();
    assert!(message.message.contains("deleted"));
}

#[tokio::test]
async fn create_task_relation_maps_already_exists_error() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/tasks/5/relations"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "code": 4012, "message": "The task relation already exists."
        })))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let err = client
        .create_task_relation(5, 9, RelationKind::Related)
        .await
        .unwrap_err();
    let Error::Api {
        status,
        code,
        message,
        ..
    } = err
    else {
        panic!("expected API error, got {err:?}");
    };
    assert_eq!(status, 409);
    assert_eq!(code, Some(4012));
    assert!(message.contains("already exists"));
}

#[tokio::test]
async fn create_task_relation_maps_invalid_task_to_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/tasks/999/relations"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "code": 4002, "message": "The task does not exist."
        })))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let err = client
        .create_task_relation(999, 9, RelationKind::Related)
        .await
        .unwrap_err();
    let Error::Api { kind, .. } = err else {
        panic!("expected API error, got {err:?}");
    };
    assert_eq!(kind, ApiErrorKind::NotFound);
}

#[tokio::test]
async fn get_task_decodes_related_tasks() {
    let server = MockServer::start().await;
    let mut body = task_json(7, "parent", false);
    body["related_tasks"] = json!({
        "subtask": [task_json(11, "child a", false), task_json(12, "child b", true)],
        "blocking": [task_json(9, "blocker", false)]
    });
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let task = client.get_task(7).await.unwrap();
    let related = task.related_tasks.expect("related_tasks should be parsed");
    assert_eq!(related["subtask"].len(), 2);
    assert_eq!(related["blocking"][0].id, 9);
}

// ----- task reminders -----------------------------------------------------------

#[tokio::test]
async fn update_task_replaces_reminders_via_read_merge_write() {
    let server = MockServer::start().await;
    // Current task state: a title to preserve and one old reminder.
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "title": "Keep me", "done": false, "project_id": 3,
            "reminders": [{"reminder": "2026-01-01T09:00:00Z"}]
        })))
        .expect(1)
        .mount(&server)
        .await;
    // The write must contain the merged object: kept title, replaced
    // reminders array (not appended to).
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/9"))
        .and(body_json(json!({
            "id": 9, "title": "Keep me", "done": false, "project_id": 3,
            "reminders": [
                {"reminder": "2026-07-01T09:00:00Z"},
                {"relative_period": -3600, "relative_to": "due_date"}
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "title": "Keep me", "done": false, "project_id": 3,
            "reminders": [
                {"reminder": "2026-07-01T09:00:00Z"},
                {"relative_period": -3600, "relative_to": "due_date"}
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let patch = TaskUpdate {
        reminders: Some(vec![
            TaskReminder {
                reminder: Some("2026-07-01T09:00:00Z".into()),
                relative_period: None,
                relative_to: None,
            },
            TaskReminder {
                reminder: None,
                relative_period: Some(-3600),
                relative_to: Some("due_date".into()),
            },
        ]),
        ..Default::default()
    };
    let task = client.update_task(9, &patch).await.unwrap();
    assert_eq!(task.reminders.unwrap().len(), 2);
}

#[tokio::test]
async fn append_task_reminder_uses_single_fetch_and_preserves_unknown_fields() {
    let server = MockServer::start().await;
    // Exactly one GET: the appended-to list and the merged write body must
    // come from the same fetch. The unknown field must survive the write.
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "title": "Keep", "done": false, "project_id": 3,
            "some_future_field": {"keep": true},
            "reminders": [{"reminder": "2026-01-01T09:00:00Z"}]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/9"))
        .and(body_json(json!({
            "id": 9, "title": "Keep", "done": false, "project_id": 3,
            "some_future_field": {"keep": true},
            "reminders": [
                {"reminder": "2026-01-01T09:00:00Z"},
                {"reminder": "2026-07-01T09:00:00Z"}
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "title": "Keep", "done": false, "project_id": 3,
            "reminders": [
                {"reminder": "2026-01-01T09:00:00Z"},
                {"reminder": "2026-07-01T09:00:00Z"}
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let reminder = TaskReminder {
        reminder: Some("2026-07-01T09:00:00Z".into()),
        relative_period: None,
        relative_to: None,
    };
    let task = client.append_task_reminder(9, &reminder).await.unwrap();
    assert_eq!(task.reminders.unwrap().len(), 2);
}

#[tokio::test]
async fn append_task_reminder_starts_list_when_absent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/tasks/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "title": "t", "done": false, "project_id": 3,
            "reminders": null
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/tasks/9"))
        .and(body_json(json!({
            "id": 9, "title": "t", "done": false, "project_id": 3,
            "reminders": [{"reminder": "2026-07-01T09:00:00Z"}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "title": "t", "done": false, "project_id": 3,
            "reminders": [{"reminder": "2026-07-01T09:00:00Z"}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let reminder = TaskReminder {
        reminder: Some("2026-07-01T09:00:00Z".into()),
        relative_period: None,
        relative_to: None,
    };
    let task = client.append_task_reminder(9, &reminder).await.unwrap();
    assert_eq!(task.reminders.unwrap().len(), 1);
}
