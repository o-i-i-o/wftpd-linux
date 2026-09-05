//! 临时验证：argon2 0.6 (password-hash 0.6) 迁移后哈希/校验行为与既有用户库兼容。

use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};

/// 与 common/src/users.rs 相同的校验路径
fn verify_password(password: &str, hash: &str) -> bool {
    let parsed_hash = match PasswordHash::new(hash) {
        Ok(h) => h,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

#[test]
fn new_hash_roundtrip() {
    let hash = Argon2::default()
        .hash_password(b"secret-pass")
        .unwrap()
        .to_string();
    assert!(hash.starts_with("$argon2id$v=19$"));
    assert!(verify_password("secret-pass", &hash));
    assert!(!verify_password("wrong-pass", &hash));
}

#[test]
fn phc_format_unchanged_explicit_salt() {
    // 旧版 (argon2 0.5 + SaltString) 产出的同样是标准 PHC 串；
    // 用带显式盐的哈希证明解析/校验路径对既有 users.json 中的哈希仍然有效。
    let old_style = Argon2::default()
        .hash_password_with_salt(b"secret-pass", b"example-salt-0k")
        .unwrap()
        .to_string();
    assert!(old_style.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
    assert!(verify_password("secret-pass", &old_style));
    assert!(!verify_password("wrong-pass", &old_style));
}
