# 发布选读

GitHub Release 由 `vMAJOR.MINOR.PATCH` 标签触发，并自动生成 Windows x64 安装包、macOS DMG 和 `SHA256SUMS.txt`。自动创建的版本默认标记为 Prerelease。

## 发布步骤

1. 将准备发布的改动合并到 `main`。
2. 在以下文件中同步更新版本号：
   - `package.json`
   - `src-tauri/Cargo.toml`
   - `src-tauri/tauri.conf.json`
3. 本地执行发布前检查：

   ```bash
   npm ci
   npm run check
   npm test
   cargo test --manifest-path src-tauri/Cargo.toml
   node scripts/check-release-version.mjs v0.1.1
   ```

4. 提交版本变更并创建带说明的标签：

   ```bash
   git tag -a v0.1.1 -m "选读 v0.1.1"
   git push origin v0.1.1
   ```

5. 等待 GitHub Actions 的 `Release` 工作流完成。
6. 手工验证 Windows 安装、卸载、托盘和选区朗读，以及 macOS 挂载、启动和辅助功能授权。
7. 验收通过后，在 GitHub Release 页面取消 `Set as a pre-release`，转为正式版本。

## 重新构建

需要重新构建已有标签时，在 GitHub Actions 中手动运行 `Release`，输入现有标签。工作流会覆盖该 Release 的同名附件，不会创建重复版本。

当前产物使用 Windows 未签名安装包和 macOS ad-hoc 签名；正式签名、公证和自动更新将在后续版本中配置。
