# Iced Demo - Rust GUI 示例项目

这是一个使用 [Iced](https://github.com/iced-rs/iced) Rust GUI 工具包创建的示例项目，展示了基础组件的使用方法。

## 功能特性

✅ **文本输入** - 实时显示用户输入内容  
✅ **按钮** - 多个交互式按钮  
✅ **计数器** - 增加/减少数字  
✅ **模态框** - 弹出对话框（暂未启用）  
✅ **滚动区域** - 消息日志显示  
✅ **响应式布局** - 使用 Column 和 Row 布局  
✅ **中文支持** - 内置中文字体，完美显示中文  

## 中文字体支持

本项目已配置中文字体支持，确保中文正常显示。

### 字体文件位置
```
fonts/NotoSansSC-Regular.ttf
```

### 如果中文显示异常
1. 检查字体文件是否存在：`ls -lh fonts/NotoSansSC-Regular.ttf`
2. 设置中文字体环境变量：
   ```bash
   export LANG=zh_CN.UTF-8
   export LC_ALL=zh_CN.UTF-8
   ```
3. 重新运行程序

## 技术栈

- **Iced 0.13** - 跨平台 Rust GUI 库
- **Tokio** - 异步运行时
- **Rust 2024 Edition** - 最新语言规范

## 运行方法

### 方式一：直接运行编译后的程序
```bash
cd /home/GGFWZX/Desktop/wftpg/Iced-demo
./target/debug/iced-demo
```

### 方式二：使用 Cargo 运行
```bash
cd /home/GGFWZX/Desktop/wftpg/Iced-demo
cargo run
```

### 方式三：发布模式运行（优化版本）
```bash
cargo build --release
./target/release/iced-demo
```

## 组件说明

### 1. 文本输入框
- 位置：顶部
- 功能：接收用户输入并实时显示
- Message: `TextInputChanged(String)`

### 2. 按钮组
- **点击我** - 记录按钮点击事件到日志
- **增加** - 计数器 +1
- **减少** - 计数器 -1
- **显示模态框** - 预留功能

### 3. 计数器显示
- 显示当前计数值
- 初始值为 0

### 4. 消息日志
- 位于底部滚动区域
- 显示所有操作记录
- 固定高度 150px

## 代码结构

```
src/main.rs
├── Message 枚举 - 定义所有可能的用户交互
├── IcedDemo 结构体 - 应用状态管理
│   ├── new() - 初始化
│   ├── update() - 处理消息和状态更新
│   └── view() - 渲染 UI 界面
└── 辅助函数
    └── rule() - 创建水平分隔线
```

## 核心概念

### Message 模式
Iced 使用 Elm 架构的 Message 模式：
1. 用户触发交互 → 产生 Message
2. `update()` 处理 Message → 更新状态
3. `view()` 根据状态 → 重新渲染 UI

### 布局系统
- **Column** - 垂直布局容器
- **Row** - 水平布局容器
- **Container** - 可自定义样式的容器
- **Scrollable** - 可滚动区域

### 样式定制
```rust
container(widget)
    .width(Length::Fixed(400.0))
    .height(Length::Fixed(300.0))
    .style(|_| container::Style {
        background: Some(iced::Background::Color(...)),
        border: iced::border::Border { ... },
        ..container::Style::default()
    })
```

## 扩展建议

### 可以添加的功能
1. ✅ 表格组件（使用 Column+Row 模拟）
2. ✅ 完整的模态框功能
3. ✅ 文件选择器
4. ✅ 主题切换
5. ✅ 数据绑定
6. ✅ 异步任务处理

### 学习资源
- [Iced 官方文档](https://docs.rs/iced/)
- [Iced 示例仓库](https://github.com/iced-rs/iced/tree/master/examples)
- [Iced 教程](https://github.com/iced-rs/iced/blob/master/docs/guide.md)

## 注意事项

⚠️ **模态框功能** - 当前版本模态框功能已预留但未完全实现，因为 Iced 0.13 的 overlay API 有所变化

⚠️ **性能** - Debug 版本体积较大 (约 253MB)，建议使用 Release 模式运行

⚠️ **依赖下载** - 首次编译需要下载大量依赖，请确保网络连接正常

## 故障排除

### 问题：编译时提示找不到某些模块
**解决**：确保使用的是 Iced 0.13 版本，API 可能与其他版本不兼容

### 问题：运行时窗口无法显示
**解决**：检查是否有 X11/Wayland 显示服务器支持

### 问题：依赖下载慢
**解决**：可以配置 Rust 镜像源加速下载

## 项目信息

- **创建时间**: 2026-03-31
- **Iced 版本**: 0.13.1
- **Rust Edition**: 2024
- **许可**: MIT

---

**享受 Rust GUI 编程的乐趣！** 🚀
