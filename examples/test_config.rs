use std::path::Path;
use wftpg::core::config::Config;

fn main() {
    let config_path = Path::new("/etc/wftpg/config.toml");
    
    match Config::load(config_path) {
        Ok(config) => {
            println!("✓ 配置文件加载成功！");
            println!("\nFTP 配置:");
            println!("  启用：{}", config.ftp.enabled);
            println!("  绑定 IP: {}", config.ftp.bind_ip);
            println!("  端口：{}", config.ftp.port);
            
            println!("\nSFTP 配置:");
            println!("  启用：{}", config.sftp.enabled);
            println!("  绑定 IP: {}", config.sftp.bind_ip);
            println!("  端口：{}", config.sftp.port);
            
            println!("\n安全配置:");
            println!("  最大连接数：{}", config.security.max_connections);
            println!("  连接超时：{}s", config.security.connection_timeout);
            println!("  空闲超时：{}s", config.security.idle_timeout);
        }
        Err(e) => {
            eprintln!("✗ 配置文件加载失败：{}", e);
            std::process::exit(1);
        }
    }
}
