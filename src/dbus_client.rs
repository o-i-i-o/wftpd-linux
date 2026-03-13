use zbus::Connection;

#[zbus::proxy(
    interface = "com.wftpg.Config",
    default_service = "com.wftpg",
    default_path = "/com/wftpg/Config"
)]
trait Config {
    fn ReadConfig(&self) -> zbus::Result<String>;
    fn WriteConfig(&self, content: &str) -> zbus::Result<()>;
    fn ReadUsers(&self) -> zbus::Result<String>;
    fn WriteUsers(&self, content: &str) -> zbus::Result<()>;
    fn ConfigExists(&self) -> zbus::Result<bool>;
    fn UsersExists(&self) -> zbus::Result<bool>;
    fn WriteAuditLog(&self, user: &str, action: &str, target: &str, details: &str) -> zbus::Result<()>;
}

pub fn read_config_via_dbus() -> Result<String, String> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| format!("创建运行时失败: {}", e))?;
    rt.block_on(async {
        let connection = Connection::system().await
            .map_err(|e| format!("连接D-Bus失败: {}", e))?;
        let proxy = ConfigProxy::new(&connection).await
            .map_err(|e| format!("创建代理失败: {}", e))?;
        proxy.ReadConfig().await
            .map_err(|e| format!("读取配置失败: {}", e))
    })
}

pub fn write_config_via_dbus(content: &str) -> Result<(), String> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| format!("创建运行时失败: {}", e))?;
    rt.block_on(async {
        let connection = Connection::system().await
            .map_err(|e| format!("连接D-Bus失败: {}", e))?;
        let proxy = ConfigProxy::new(&connection).await
            .map_err(|e| format!("创建代理失败: {}", e))?;
        proxy.WriteConfig(content).await
            .map_err(|e| format!("写入配置失败: {}", e))
    })
}

pub fn read_users_via_dbus() -> Result<String, String> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| format!("创建运行时失败: {}", e))?;
    rt.block_on(async {
        let connection = Connection::system().await
            .map_err(|e| format!("连接D-Bus失败: {}", e))?;
        let proxy = ConfigProxy::new(&connection).await
            .map_err(|e| format!("创建代理失败: {}", e))?;
        proxy.ReadUsers().await
            .map_err(|e| format!("读取用户配置失败: {}", e))
    })
}

pub fn write_users_via_dbus(content: &str) -> Result<(), String> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| format!("创建运行时失败: {}", e))?;
    rt.block_on(async {
        let connection = Connection::system().await
            .map_err(|e| format!("连接D-Bus失败: {}", e))?;
        let proxy = ConfigProxy::new(&connection).await
            .map_err(|e| format!("创建代理失败: {}", e))?;
        proxy.WriteUsers(content).await
            .map_err(|e| format!("写入用户配置失败: {}", e))
    })
}

pub fn write_audit_log(user: &str, action: &str, target: &str, details: &str) -> Result<(), String> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| format!("创建运行时失败: {}", e))?;
    let user = user.to_string();
    let action = action.to_string();
    let target = target.to_string();
    let details = details.to_string();
    rt.block_on(async {
        let connection = Connection::system().await
            .map_err(|e| format!("连接D-Bus失败: {}", e))?;
        let proxy = ConfigProxy::new(&connection).await
            .map_err(|e| format!("创建代理失败: {}", e))?;
        proxy.WriteAuditLog(&user, &action, &target, &details).await
            .map_err(|e| format!("写入审计日志失败: {}", e))
    })
}
