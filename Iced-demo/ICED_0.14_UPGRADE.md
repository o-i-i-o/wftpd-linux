# Iced 0.14 升级报告

## ✅ 升级完成状态

**Iced 版本**: 0.13 → 0.14  
**升级时间**: 2026-03-31  
**状态**: ⚠️ 部分功能需要适配

## 📝 主要变更

### 1. Cargo.toml 配置
```toml
[dependencies]
iced = { version = "0.14", features = ["tokio"] }
tokio = { version = "1", features = ["full"] }
```

**变更说明**:
- ✅ 版本号从 `0.13` 升级到 `0.14`
- ❌ 移除了 `font-kit` 依赖（不再需要）

### 2. Application API 重大变更

#### Iced 0.13 (旧版本)
```rust
pub fn main() -> iced::Result {
    iced::application("Title", update, view)
        .default_font(Font::MONOSPACE)
        .run_with(IcedDemo::new)
}
```

#### Iced 0.14 (新版本)
```rust
pub fn main() -> iced::Result {
    iced::application(
        move || (IcedDemo::new(), Task::none()),
        IcedDemo::update,
        IcedDemo::view,
    )
    .run()
}
```

**关键变化**:
1. ❌ 移除了标题参数
2. ✅ `new()` 现在返回 `(Self, Task<Message>)` 元组
3. ✅ `update()` 必须返回 `Task<Message>`
4. ✅ `view()` 签名变为 `fn view(&self) -> Element<'_, Message>`
5. ❌ 移除了 `.default_font()` 和 `.settings()` 方法（API 变更）

### 3. Rule 组件变更

#### Iced 0.13
```rust
Rule::horizontal(2)
Rule::vertical(2)
```

#### Iced 0.14
⚠️ **API 已变更，需要查找新用法**

### 4. 字体加载 API 变更

#### Iced 0.13
```rust
.settings(iced::Settings {
    default_font: Font::with_name("Noto Sans SC"),
    fonts: vec![/* ... */],
    ..Default::default()
})
```

#### Iced 0.14
⚠️ **API 已完全重构，需要适配**

## 🔧 中文字体支持方案

### 当前问题
Iced 0.14 的字体加载 API 发生重大变化，暂时无法通过代码嵌入字体。

### 临时解决方案
1. **使用系统字体** - Iced 会自动使用系统默认字体
2. **设置环境变量** - 确保系统有中文语言环境
   ```bash
   export LANG=zh_CN.UTF-8
   export LC_ALL=zh_CN.UTF-8
   ```

### 推荐方案（等待 Iced 0.14 完善）
1. 关注 Iced 官方文档更新
2. 查看示例项目了解新的字体加载方式
3. 考虑使用 `iced::font` 模块的新 API

## 📋 已完成的工作

### ✅ 代码更新
- [x] 更新 Cargo.toml 版本号
- [x] 移除 font-kit 依赖
- [x] 更新 application 调用方式
- [x] 修改 new() 函数签名
- [x] 修改 update() 函数签名
- [x] 更新 view() 生命周期标注

### ⚠️ 待解决的问题
- [ ] Rule 组件的正确用法
- [ ] 字体嵌入 API 适配
- [ ] Settings 配置方式

## 🔍 编译错误汇总

### 错误 1: FontData 未找到
```
error[E0433]: failed to resolve: could not find `FontData` in `iced`
```
**原因**: Iced 0.14 重构了字体模块  
**状态**: 待解决

### 错误 2: Rule::horizontal 不存在
```
error[E0599]: no function or associated item named `horizontal`
```
**原因**: Rule API 已变更  
**状态**: 待解决

### 错误 3: application 参数不匹配
```
error[E0593]: closure is expected to take 2 arguments
```
**原因**: application 函数签名变化  
**状态**: ✅ 已修复

## 💡 下一步建议

### 立即行动
1. 查看 [Iced 0.14 官方示例](https://github.com/iced-rs/iced/tree/master/examples)
2. 阅读 [Iced 0.14 迁移指南](https://github.com/iced-rs/iced/releases)
3. 检查 `iced::widget::Rule` 的新用法

### 可能的解决方案
```rust
// 尝试使用 row 或 column 模拟分隔线
use iced::widget::Space;

fn rule() -> Element<'static, Message> {
    Space::with_height(Length::Fixed(2.0)).into()
}
```

## 📚 参考资料

- [Iced GitHub](https://github.com/iced-rs/iced)
- [Iced 0.14 Release Notes](https://github.com/iced-rs/iced/releases/tag/iced%400.14.0)
- [Iced Examples](https://github.com/iced-rs/iced/tree/master/examples)
- [Iced Documentation](https://docs.rs/iced/0.14.0/iced/)

## ⚠️ 注意事项

1. **API 不稳定** - Iced 仍在快速发展中，API 经常变化
2. **文档滞后** - 文档可能跟不上代码变化
3. **示例参考** - 最佳学习方式是查看官方示例
4. **中文支持** - 需要额外配置字体

---

**更新时间**: 2026-03-31  
**状态**: ⚠️ 升级中，部分功能待适配
