use pgmount_core::db::migrate;
use pgmount_core::db::pool::create_pool;
use pgmount_core::db::queries::workspace as ws_queries;
use pgmount_core::db::types::{WorkspaceFile, WorkspaceLayout};

fn connection_string() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "host=postgres user=pgmount password=pgmount dbname=testdb".to_string())
}

async fn get_pool() -> deadpool_postgres::Pool {
    let pool = create_pool(&connection_string(), 30).unwrap();
    migrate::run_migrations(&pool).await.unwrap();

    // Clean up any leftover test workspaces
    let _ = ws_queries::delete_workspace(&pool, "ws-test-1").await;
    let _ = ws_queries::delete_workspace(&pool, "ws-test-2").await;
    let _ = ws_queries::delete_workspace(&pool, "ws-seed-test").await;

    pool
}

fn now_ns() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i64
}

#[tokio::test]
async fn test_create_and_get_workspace() {
    let pool = get_pool().await;

    let layout = WorkspaceLayout {
        auto_dirs: vec![".claude".into(), ".claude/memory".into()],
        seed_files: Default::default(),
    };

    ws_queries::create_workspace(&pool, "ws-test-1", Some("Test Workspace"), &layout)
        .await
        .unwrap();

    let ws = ws_queries::get_workspace(&pool, "ws-test-1").await.unwrap();
    assert_eq!(ws.id, "ws-test-1");
    assert_eq!(ws.display_name, Some("Test Workspace".to_string()));
    assert_eq!(ws.config.auto_dirs.len(), 2);

    // Cleanup
    ws_queries::delete_workspace(&pool, "ws-test-1").await.unwrap();
}

#[tokio::test]
async fn test_list_workspaces() {
    let pool = get_pool().await;

    let layout = WorkspaceLayout::default();
    ws_queries::create_workspace(&pool, "ws-test-1", Some("One"), &layout).await.unwrap();
    ws_queries::create_workspace(&pool, "ws-test-2", Some("Two"), &layout).await.unwrap();

    let workspaces = ws_queries::list_workspaces(&pool).await.unwrap();
    assert!(workspaces.len() >= 2);
    assert!(workspaces.iter().any(|w| w.id == "ws-test-1"));
    assert!(workspaces.iter().any(|w| w.id == "ws-test-2"));

    ws_queries::delete_workspace(&pool, "ws-test-1").await.unwrap();
    ws_queries::delete_workspace(&pool, "ws-test-2").await.unwrap();
}

#[tokio::test]
async fn test_create_and_get_file() {
    let pool = get_pool().await;
    let layout = WorkspaceLayout::default();
    ws_queries::create_workspace(&pool, "ws-test-1", None, &layout).await.unwrap();
    ws_queries::seed_from_config(&pool, "ws-test-1", &layout).await.unwrap();

    let now = now_ns();
    let file = WorkspaceFile {
        workspace_id: "ws-test-1".to_string(),
        path: "/hello.txt".to_string(),
        parent_path: "/".to_string(),
        name: "hello.txt".to_string(),
        is_dir: false,
        content: Some(b"hello world".to_vec()),
        mode: 0o100644,
        size: 11,
        mtime_ns: now,
        ctime_ns: now,
        atime_ns: now,
        nlink: 1,
        uid: 1000,
        gid: 1000,
    };

    ws_queries::create_file(&pool, &file).await.unwrap();

    let fetched = ws_queries::get_file(&pool, "ws-test-1", "/hello.txt").await.unwrap();
    assert_eq!(fetched.name, "hello.txt");
    assert_eq!(fetched.content, Some(b"hello world".to_vec()));
    assert_eq!(fetched.size, 11);
    assert!(!fetched.is_dir);

    ws_queries::delete_workspace(&pool, "ws-test-1").await.unwrap();
}

#[tokio::test]
async fn test_create_file_exists_error() {
    let pool = get_pool().await;
    let layout = WorkspaceLayout::default();
    ws_queries::create_workspace(&pool, "ws-test-1", None, &layout).await.unwrap();
    ws_queries::seed_from_config(&pool, "ws-test-1", &layout).await.unwrap();

    let now = now_ns();
    let file = WorkspaceFile {
        workspace_id: "ws-test-1".to_string(),
        path: "/dup.txt".to_string(),
        parent_path: "/".to_string(),
        name: "dup.txt".to_string(),
        is_dir: false,
        content: Some(b"first".to_vec()),
        mode: 0o100644,
        size: 5,
        mtime_ns: now,
        ctime_ns: now,
        atime_ns: now,
        nlink: 1,
        uid: 1000,
        gid: 1000,
    };

    ws_queries::create_file(&pool, &file).await.unwrap();
    let err = ws_queries::create_file(&pool, &file).await.unwrap_err();
    assert!(matches!(err, pgmount_core::error::FsError::FileExists));

    ws_queries::delete_workspace(&pool, "ws-test-1").await.unwrap();
}

#[tokio::test]
async fn test_list_children() {
    let pool = get_pool().await;
    let layout = WorkspaceLayout::default();
    ws_queries::create_workspace(&pool, "ws-test-1", None, &layout).await.unwrap();
    ws_queries::seed_from_config(&pool, "ws-test-1", &layout).await.unwrap();

    let now = now_ns();

    // Create a directory
    let dir = WorkspaceFile {
        workspace_id: "ws-test-1".to_string(),
        path: "/mydir".to_string(),
        parent_path: "/".to_string(),
        name: "mydir".to_string(),
        is_dir: true,
        content: None,
        mode: 0o40755,
        size: 0,
        mtime_ns: now,
        ctime_ns: now,
        atime_ns: now,
        nlink: 2,
        uid: 1000,
        gid: 1000,
    };
    ws_queries::create_file(&pool, &dir).await.unwrap();

    // Create files in it
    for name in ["a.txt", "b.txt", "c.txt"] {
        let f = WorkspaceFile {
            workspace_id: "ws-test-1".to_string(),
            path: format!("/mydir/{}", name),
            parent_path: "/mydir".to_string(),
            name: name.to_string(),
            is_dir: false,
            content: Some(name.as_bytes().to_vec()),
            mode: 0o100644,
            size: name.len() as i64,
            mtime_ns: now,
            ctime_ns: now,
            atime_ns: now,
            nlink: 1,
            uid: 1000,
            gid: 1000,
        };
        ws_queries::create_file(&pool, &f).await.unwrap();
    }

    let children = ws_queries::list_children(&pool, "ws-test-1", "/mydir").await.unwrap();
    assert_eq!(children.len(), 3);
    let names: Vec<&str> = children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["a.txt", "b.txt", "c.txt"]);

    ws_queries::delete_workspace(&pool, "ws-test-1").await.unwrap();
}

#[tokio::test]
async fn test_update_file_content() {
    let pool = get_pool().await;
    let layout = WorkspaceLayout::default();
    ws_queries::create_workspace(&pool, "ws-test-1", None, &layout).await.unwrap();
    ws_queries::seed_from_config(&pool, "ws-test-1", &layout).await.unwrap();

    let now = now_ns();
    let file = WorkspaceFile {
        workspace_id: "ws-test-1".to_string(),
        path: "/data.bin".to_string(),
        parent_path: "/".to_string(),
        name: "data.bin".to_string(),
        is_dir: false,
        content: Some(b"initial".to_vec()),
        mode: 0o100644,
        size: 7,
        mtime_ns: now,
        ctime_ns: now,
        atime_ns: now,
        nlink: 1,
        uid: 1000,
        gid: 1000,
    };
    ws_queries::create_file(&pool, &file).await.unwrap();

    ws_queries::update_file_content(&pool, "ws-test-1", "/data.bin", b"updated content", now + 1000)
        .await
        .unwrap();

    let fetched = ws_queries::get_file(&pool, "ws-test-1", "/data.bin").await.unwrap();
    assert_eq!(fetched.content, Some(b"updated content".to_vec()));
    assert_eq!(fetched.size, 15);

    ws_queries::delete_workspace(&pool, "ws-test-1").await.unwrap();
}

#[tokio::test]
async fn test_delete_file() {
    let pool = get_pool().await;
    let layout = WorkspaceLayout::default();
    ws_queries::create_workspace(&pool, "ws-test-1", None, &layout).await.unwrap();
    ws_queries::seed_from_config(&pool, "ws-test-1", &layout).await.unwrap();

    let now = now_ns();
    let file = WorkspaceFile {
        workspace_id: "ws-test-1".to_string(),
        path: "/todelete.txt".to_string(),
        parent_path: "/".to_string(),
        name: "todelete.txt".to_string(),
        is_dir: false,
        content: Some(b"bye".to_vec()),
        mode: 0o100644,
        size: 3,
        mtime_ns: now,
        ctime_ns: now,
        atime_ns: now,
        nlink: 1,
        uid: 1000,
        gid: 1000,
    };
    ws_queries::create_file(&pool, &file).await.unwrap();
    ws_queries::delete_file(&pool, "ws-test-1", "/todelete.txt").await.unwrap();

    let err = ws_queries::get_file(&pool, "ws-test-1", "/todelete.txt").await.unwrap_err();
    assert!(matches!(err, pgmount_core::error::FsError::NotFound));

    ws_queries::delete_workspace(&pool, "ws-test-1").await.unwrap();
}

#[tokio::test]
async fn test_delete_nonempty_dir() {
    let pool = get_pool().await;
    let layout = WorkspaceLayout::default();
    ws_queries::create_workspace(&pool, "ws-test-1", None, &layout).await.unwrap();
    ws_queries::seed_from_config(&pool, "ws-test-1", &layout).await.unwrap();

    let now = now_ns();
    let dir = WorkspaceFile {
        workspace_id: "ws-test-1".to_string(),
        path: "/notempty".to_string(),
        parent_path: "/".to_string(),
        name: "notempty".to_string(),
        is_dir: true,
        content: None,
        mode: 0o40755,
        size: 0,
        mtime_ns: now,
        ctime_ns: now,
        atime_ns: now,
        nlink: 2,
        uid: 1000,
        gid: 1000,
    };
    ws_queries::create_file(&pool, &dir).await.unwrap();

    let file = WorkspaceFile {
        workspace_id: "ws-test-1".to_string(),
        path: "/notempty/child.txt".to_string(),
        parent_path: "/notempty".to_string(),
        name: "child.txt".to_string(),
        is_dir: false,
        content: Some(b"x".to_vec()),
        mode: 0o100644,
        size: 1,
        mtime_ns: now,
        ctime_ns: now,
        atime_ns: now,
        nlink: 1,
        uid: 1000,
        gid: 1000,
    };
    ws_queries::create_file(&pool, &file).await.unwrap();

    let err = ws_queries::delete_directory(&pool, "ws-test-1", "/notempty").await.unwrap_err();
    assert!(matches!(err, pgmount_core::error::FsError::DirectoryNotEmpty));

    ws_queries::delete_workspace(&pool, "ws-test-1").await.unwrap();
}

#[tokio::test]
async fn test_rename_file() {
    let pool = get_pool().await;
    let layout = WorkspaceLayout::default();
    ws_queries::create_workspace(&pool, "ws-test-1", None, &layout).await.unwrap();
    ws_queries::seed_from_config(&pool, "ws-test-1", &layout).await.unwrap();

    let now = now_ns();
    let file = WorkspaceFile {
        workspace_id: "ws-test-1".to_string(),
        path: "/old.txt".to_string(),
        parent_path: "/".to_string(),
        name: "old.txt".to_string(),
        is_dir: false,
        content: Some(b"rename me".to_vec()),
        mode: 0o100644,
        size: 9,
        mtime_ns: now,
        ctime_ns: now,
        atime_ns: now,
        nlink: 1,
        uid: 1000,
        gid: 1000,
    };
    ws_queries::create_file(&pool, &file).await.unwrap();

    ws_queries::rename_file(&pool, "ws-test-1", "/old.txt", "/new.txt", "/", "new.txt")
        .await
        .unwrap();

    let err = ws_queries::get_file(&pool, "ws-test-1", "/old.txt").await.unwrap_err();
    assert!(matches!(err, pgmount_core::error::FsError::NotFound));

    let renamed = ws_queries::get_file(&pool, "ws-test-1", "/new.txt").await.unwrap();
    assert_eq!(renamed.name, "new.txt");
    assert_eq!(renamed.content, Some(b"rename me".to_vec()));

    ws_queries::delete_workspace(&pool, "ws-test-1").await.unwrap();
}

#[tokio::test]
async fn test_seed_from_config() {
    let pool = get_pool().await;

    let layout = WorkspaceLayout {
        auto_dirs: vec![".claude".into(), ".claude/memory".into()],
        seed_files: [
            (".claude/settings.json".into(), "{\"model\": \"sonnet\"}".into()),
        ]
        .into(),
    };

    ws_queries::create_workspace(&pool, "ws-seed-test", Some("Seed Test"), &layout)
        .await
        .unwrap();
    ws_queries::seed_from_config(&pool, "ws-seed-test", &layout)
        .await
        .unwrap();

    // Root should exist
    let root = ws_queries::get_file(&pool, "ws-seed-test", "/").await.unwrap();
    assert!(root.is_dir);

    // .claude directory should exist
    let claude_dir = ws_queries::get_file(&pool, "ws-seed-test", "/.claude").await.unwrap();
    assert!(claude_dir.is_dir);

    // .claude/memory directory should exist
    let memory_dir = ws_queries::get_file(&pool, "ws-seed-test", "/.claude/memory").await.unwrap();
    assert!(memory_dir.is_dir);

    // seed file should exist
    let settings = ws_queries::get_file(&pool, "ws-seed-test", "/.claude/settings.json")
        .await
        .unwrap();
    assert!(!settings.is_dir);
    assert_eq!(
        settings.content,
        Some(b"{\"model\": \"sonnet\"}".to_vec())
    );

    // Root children should include .claude
    let children = ws_queries::list_children(&pool, "ws-seed-test", "/").await.unwrap();
    assert!(children.iter().any(|c| c.name == ".claude"));

    ws_queries::delete_workspace(&pool, "ws-seed-test").await.unwrap();
}

#[tokio::test]
async fn test_cascade_delete() {
    let pool = get_pool().await;
    let layout = WorkspaceLayout {
        auto_dirs: vec![".claude".into()],
        seed_files: [(".claude/test.txt".into(), "data".into())].into(),
    };

    ws_queries::create_workspace(&pool, "ws-test-1", None, &layout).await.unwrap();
    ws_queries::seed_from_config(&pool, "ws-test-1", &layout).await.unwrap();

    // Verify files exist
    ws_queries::get_file(&pool, "ws-test-1", "/.claude/test.txt").await.unwrap();

    // Delete workspace — should cascade to files
    ws_queries::delete_workspace(&pool, "ws-test-1").await.unwrap();

    // Files should be gone
    let err = ws_queries::get_file(&pool, "ws-test-1", "/.claude/test.txt").await.unwrap_err();
    assert!(matches!(err, pgmount_core::error::FsError::NotFound));
}
