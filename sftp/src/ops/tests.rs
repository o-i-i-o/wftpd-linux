//! SFTP 协议操作单元测试。

use super::extended::parse_digest_request;
use super::*;
use russh_sftp::protocol::{OpenFlags, Packet};
use russh_sftp::server::Handler;

fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(fut)
}

struct TestEnv {
    dir: tempfile::TempDir,
    manager: Arc<StdMutex<UserManager>>,
    handler: SftpFileHandler,
}

fn setup() -> TestEnv {
    let dir = tempfile::tempdir().unwrap();
    let mut manager = UserManager::new();
    manager
        .add_user(
            "tester".into(),
            "pw",
            dir.path().to_string_lossy().into_owned(),
            Permissions::full(),
            false,
        )
        .unwrap();
    let manager = Arc::new(StdMutex::new(manager));
    let handler = SftpFileHandler::new(
        "tester".into(),
        dir.path().to_string_lossy().into_owned(),
        Arc::clone(&manager),
        Arc::new(StdMutex::new(FileLogger::new("/tmp", 0))),
        Arc::new(QuotaCache::new()),
        "127.0.0.1".into(),
    );
    TestEnv {
        dir,
        manager,
        handler,
    }
}

fn set_perms(env: &TestEnv, f: impl FnOnce(&mut Permissions)) {
    let user = env
        .manager
        .lock()
        .unwrap()
        .get_user("tester")
        .map(|u| u.username.clone())
        .unwrap();
    let mut perms = env
        .manager
        .lock()
        .unwrap()
        .get_user(&user)
        .unwrap()
        .permissions;
    f(&mut perms);
    env.manager
        .lock()
        .unwrap()
        .update_permissions(&user, perms)
        .unwrap();
}

// ---- resolve：词法路径锚定 ----

#[test]
fn resolve_maps_paths_into_home() {
    let env = setup();
    let home = env.dir.path().canonicalize().unwrap();

    assert_eq!(env.handler.resolve("/").unwrap(), home);
    assert_eq!(env.handler.resolve("").unwrap(), home);
    assert_eq!(env.handler.resolve("foo").unwrap(), home.join("foo"));
    assert_eq!(env.handler.resolve("/foo").unwrap(), home.join("foo"));
    assert_eq!(env.handler.resolve("./foo").unwrap(), home.join("foo"));
}

#[test]
fn resolve_accepts_real_absolute_path_with_home_prefix() {
    let env = setup();
    let home = env.dir.path().canonicalize().unwrap();
    let real = format!("{}/docs", home.display());
    assert_eq!(env.handler.resolve(&real).unwrap(), home.join("docs"));
}

#[test]
fn resolve_normalizes_dotdot_within_home() {
    let env = setup();
    let home = env.dir.path().canonicalize().unwrap();
    assert_eq!(env.handler.resolve("/a/../b").unwrap(), home.join("b"));
}

#[test]
fn resolve_rejects_escape_above_home() {
    let env = setup();
    assert_eq!(
        env.handler.resolve("/..").unwrap_err(),
        StatusCode::NoSuchFile
    );
    assert_eq!(
        env.handler.resolve("/a/../../x").unwrap_err(),
        StatusCode::NoSuchFile
    );
}

// ---- 纯工具函数 ----

#[test]
fn parse_digest_request_wire_format() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&3u32.to_be_bytes());
    buf.extend_from_slice(b"abc");
    buf.extend_from_slice(&1u64.to_be_bytes());
    buf.extend_from_slice(&2u64.to_be_bytes());
    assert_eq!(parse_digest_request(&buf), Some(("abc".to_string(), 1, 2)));
}

#[test]
fn parse_digest_request_defaults_offset_length() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&3u32.to_be_bytes());
    buf.extend_from_slice(b"abc");
    assert_eq!(parse_digest_request(&buf), Some(("abc".to_string(), 0, 0)));
}

#[test]
fn parse_digest_request_rejects_truncated_and_bad_utf8() {
    assert_eq!(parse_digest_request(&[]), None);
    assert_eq!(parse_digest_request(&[0xFF, 0xFF, 0xFF, 0xFF]), None);
    let mut buf = Vec::new();
    buf.extend_from_slice(&1u32.to_be_bytes());
    buf.push(0xFF);
    assert_eq!(parse_digest_request(&buf), None);
}

#[test]
fn io_status_maps_error_kinds() {
    use std::io::ErrorKind;
    assert_eq!(
        SftpFileHandler::io_status(&std::io::Error::from(ErrorKind::NotFound)),
        StatusCode::NoSuchFile
    );
    assert_eq!(
        SftpFileHandler::io_status(&std::io::Error::from(ErrorKind::PermissionDenied)),
        StatusCode::PermissionDenied
    );
    assert_eq!(
        SftpFileHandler::io_status(&std::io::Error::from(ErrorKind::Interrupted)),
        StatusCode::Failure
    );
}

#[test]
fn next_handle_id_is_unique() {
    let mut env = setup();
    let first = env.handler.next_handle_id();
    let second = env.handler.next_handle_id();
    assert_ne!(first, second);
    assert_eq!(first, "h1");
    assert_eq!(second, "h2");
}

#[test]
fn init_negotiates_version_and_extensions() {
    let mut env = setup();
    let version = block_on(env.handler.init(3, HashMap::new())).unwrap();
    assert_eq!(version.version, SFTP_VERSION);
    for ext in [
        "md5sum@openssh.com",
        "sha256sum@openssh.com",
        "space-available@openssh.com",
    ] {
        assert!(version.extensions.contains_key(ext), "缺少扩展 {ext}");
    }
}

// ---- open/write/read/close 生命周期 ----

#[test]
fn open_write_read_close_roundtrip() {
    let mut env = setup();

    let flags = OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE;
    let handle =
        block_on(
            env.handler
                .open(1, "note.txt".into(), flags, FileAttributes::dummy()),
        )
        .unwrap()
        .handle;
    block_on(
        env.handler
            .write(2, handle.clone(), 0, b"hello world".to_vec()),
    )
    .unwrap();
    block_on(env.handler.close(3, handle.clone())).unwrap();

    let read_handle = block_on(env.handler.open(
        4,
        "note.txt".into(),
        OpenFlags::READ,
        FileAttributes::dummy(),
    ))
    .unwrap()
    .handle;
    let data = block_on(env.handler.read(5, read_handle.clone(), 0, 5)).unwrap();
    assert_eq!(data.data, b"hello");
    let rest = block_on(env.handler.read(6, read_handle.clone(), 5, 100)).unwrap();
    assert_eq!(rest.data, b" world");
    // 读到末尾返回 EOF
    assert_eq!(
        block_on(env.handler.read(7, read_handle.clone(), 11, 10)).unwrap_err(),
        StatusCode::Eof
    );
    block_on(env.handler.close(8, read_handle)).unwrap();
}

#[test]
fn read_caps_length_at_max() {
    let mut env = setup();
    // 请求超大长度不应导致异常分配（内部钳制到 MAX_READ_LEN）
    let flags = OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE;
    let h = block_on(
        env.handler
            .open(1, "small.txt".into(), flags, FileAttributes::dummy()),
    )
    .unwrap()
    .handle;
    block_on(env.handler.write(2, h.clone(), 0, b"ab".to_vec())).unwrap();
    block_on(env.handler.close(3, h)).unwrap();

    let rh = block_on(env.handler.open(
        4,
        "small.txt".into(),
        OpenFlags::READ,
        FileAttributes::dummy(),
    ))
    .unwrap()
    .handle;
    let data = block_on(env.handler.read(5, rh.clone(), 0, u32::MAX)).unwrap();
    assert_eq!(data.data, b"ab");
    block_on(env.handler.close(6, rh)).unwrap();
}

#[test]
fn close_unknown_handle_fails() {
    let mut env = setup();
    assert_eq!(
        block_on(env.handler.close(1, "nope".into())).unwrap_err(),
        StatusCode::NoSuchFile
    );
}

// ---- 权限控制 ----

#[test]
fn open_denied_without_write_permission() {
    let mut env = setup();
    set_perms(&env, |p| p.can_write = false);
    let flags = OpenFlags::WRITE | OpenFlags::CREATE;
    assert_eq!(
        block_on(
            env.handler
                .open(1, "x.txt".into(), flags, FileAttributes::dummy())
        )
        .unwrap_err(),
        StatusCode::PermissionDenied
    );
}

#[test]
fn open_append_allowed_with_append_permission_only() {
    let mut env = setup();
    std::fs::write(env.dir.path().join("log.txt"), b"old").unwrap();
    set_perms(&env, |p| {
        p.can_write = false;
        p.can_append = true;
    });
    let h = block_on(env.handler.open(
        1,
        "log.txt".into(),
        OpenFlags::WRITE | OpenFlags::APPEND,
        FileAttributes::dummy(),
    ))
    .unwrap()
    .handle;
    block_on(env.handler.write(2, h.clone(), 3, b"new".to_vec())).unwrap();
    block_on(env.handler.close(3, h)).unwrap();
    assert_eq!(
        std::fs::read(env.dir.path().join("log.txt")).unwrap(),
        b"oldnew"
    );
}

#[test]
fn open_denied_without_read_permission() {
    let mut env = setup();
    std::fs::write(env.dir.path().join("x.txt"), b"data").unwrap();
    set_perms(&env, |p| p.can_read = false);
    assert_eq!(
        block_on(
            env.handler
                .open(1, "x.txt".into(), OpenFlags::READ, FileAttributes::dummy())
        )
        .unwrap_err(),
        StatusCode::PermissionDenied
    );
}

#[test]
fn listing_denied_without_list_permission() {
    let mut env = setup();
    set_perms(&env, |p| p.can_list = false);
    assert_eq!(
        block_on(env.handler.opendir(1, ".".into())).unwrap_err(),
        StatusCode::PermissionDenied
    );
    assert_eq!(
        block_on(env.handler.stat(1, "x".into())).unwrap_err(),
        StatusCode::PermissionDenied
    );
    assert_eq!(
        block_on(env.handler.lstat(1, "x".into())).unwrap_err(),
        StatusCode::PermissionDenied
    );
}

#[test]
fn mutating_ops_denied_without_permissions() {
    let mut env = setup();
    set_perms(&env, |p| {
        p.can_delete = false;
        p.can_mkdir = false;
        p.can_rmdir = false;
        p.can_rename = false;
    });
    assert_eq!(
        block_on(env.handler.remove(1, "x".into())).unwrap_err(),
        StatusCode::PermissionDenied
    );
    assert_eq!(
        block_on(env.handler.mkdir(1, "d".into(), FileAttributes::dummy())).unwrap_err(),
        StatusCode::PermissionDenied
    );
    assert_eq!(
        block_on(env.handler.rmdir(1, "d".into())).unwrap_err(),
        StatusCode::PermissionDenied
    );
    assert_eq!(
        block_on(env.handler.rename(1, "a".into(), "b".into())).unwrap_err(),
        StatusCode::PermissionDenied
    );
}

// ---- 目录操作 ----

#[test]
fn mkdir_readdir_rmdir_roundtrip() {
    let mut env = setup();
    std::fs::write(env.dir.path().join("a.txt"), b"1").unwrap();
    std::fs::write(env.dir.path().join("b.txt"), b"2").unwrap();

    let dir_handle = block_on(env.handler.opendir(1, ".".into())).unwrap().handle;
    let name = block_on(env.handler.readdir(2, dir_handle.clone())).unwrap();
    let names: Vec<String> = name.files.into_iter().map(|f| f.filename).collect();
    assert!(names.contains(&"a.txt".to_string()));
    assert!(names.contains(&"b.txt".to_string()));
    // 一次性枚举完毕后再次 readdir 返回 EOF
    assert_eq!(
        block_on(env.handler.readdir(3, dir_handle.clone())).unwrap_err(),
        StatusCode::Eof
    );
    block_on(env.handler.close(4, dir_handle)).unwrap();

    // mkdir → rmdir
    block_on(
        env.handler
            .mkdir(5, "newdir".into(), FileAttributes::dummy()),
    )
    .unwrap();
    assert!(env.dir.path().join("newdir").is_dir());
    block_on(env.handler.rmdir(6, "newdir".into())).unwrap();
    assert!(!env.dir.path().join("newdir").exists());
}

#[test]
fn remove_and_rename_files() {
    let mut env = setup();
    std::fs::write(env.dir.path().join("old.txt"), b"x").unwrap();

    block_on(
        env.handler
            .rename(1, "old.txt".into(), "renamed.txt".into()),
    )
    .unwrap();
    assert!(env.dir.path().join("renamed.txt").exists());

    block_on(env.handler.remove(2, "renamed.txt".into())).unwrap();
    assert!(!env.dir.path().join("renamed.txt").exists());
    assert_eq!(
        block_on(env.handler.remove(3, "renamed.txt".into())).unwrap_err(),
        StatusCode::NoSuchFile
    );
}

#[test]
fn rename_to_existing_target_fails() {
    let mut env = setup();
    std::fs::write(env.dir.path().join("src.txt"), b"s").unwrap();
    std::fs::write(env.dir.path().join("dst.txt"), b"d").unwrap();
    assert_eq!(
        block_on(env.handler.rename(1, "src.txt".into(), "dst.txt".into())).unwrap_err(),
        StatusCode::Failure
    );
}

// ---- 路径与属性 ----

#[test]
fn realpath_maps_to_virtual_root() {
    let mut env = setup();
    let root = block_on(env.handler.realpath(1, "/".into())).unwrap();
    assert_eq!(root.files[0].filename, "/");

    std::fs::create_dir_all(env.dir.path().join("docs")).unwrap();
    let docs = block_on(env.handler.realpath(2, "docs".into())).unwrap();
    assert_eq!(docs.files[0].filename, "/docs");
}

#[test]
fn stat_reports_file_size() {
    let mut env = setup();
    std::fs::write(env.dir.path().join("f.bin"), vec![0u8; 123]).unwrap();

    let attrs = block_on(env.handler.stat(1, "f.bin".into())).unwrap().attrs;
    assert_eq!(attrs.size, Some(123));
    assert!(attrs.permissions.is_some());
}

#[test]
fn stat_missing_file_returns_no_such_file() {
    let mut env = setup();
    assert_eq!(
        block_on(env.handler.stat(1, "missing.txt".into())).unwrap_err(),
        StatusCode::NoSuchFile
    );
}

// ---- openssh 扩展 ----

#[test]
fn extended_md5sum_and_sha256sum() {
    let mut env = setup();
    std::fs::write(env.dir.path().join("data.bin"), b"abc").unwrap();

    let mut req = Vec::new();
    req.extend_from_slice(&8u32.to_be_bytes());
    req.extend_from_slice(b"data.bin");
    req.extend_from_slice(&0u64.to_be_bytes());
    req.extend_from_slice(&0u64.to_be_bytes());

    let packet = block_on(
        env.handler
            .extended(1, "md5sum@openssh.com".into(), req.clone()),
    )
    .unwrap();
    let Packet::ExtendedReply(reply) = packet else {
        panic!("应返回 ExtendedReply");
    };
    assert_eq!(reply.data, b"900150983cd24fb0d6963f7d28e17f72");

    let packet = block_on(env.handler.extended(2, "sha256sum@openssh.com".into(), req)).unwrap();
    let Packet::ExtendedReply(reply) = packet else {
        panic!("应返回 ExtendedReply");
    };
    assert_eq!(
        reply.data,
        b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn extended_unknown_request_unsupported() {
    let mut env = setup();
    assert_eq!(
        block_on(env.handler.extended(1, "fancy@vendor.com".into(), vec![])).unwrap_err(),
        StatusCode::OpUnsupported
    );
}
