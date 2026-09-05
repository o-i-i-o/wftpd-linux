//! 使用 protox（纯 Rust）编译 .proto，无需系统安装 protoc。

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fds = protox::compile(["proto/wftpd.proto"], ["proto/"])?;
    tonic_build::compile_fds(fds)?;
    Ok(())
}
