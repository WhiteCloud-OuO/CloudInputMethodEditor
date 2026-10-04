# 安全政策 / Security Policy

## 支持的版本

云朵输入法还在测试阶段，只修最新版本：

| 版本 | 支持 |
|---|---|
| 最新的 0.1.x | ✅ |
| 更早 | ❌，请升级 |

## 报告漏洞

**不要在公开 issue 里写漏洞细节。** 请用 GitHub 的私密漏洞报告：仓库页面「Security」→「Report a vulnerability」。

收到后 3 天内回复，确认后在下一个版本修复并在更新日志里致谢（除非你不愿意署名）；不算漏洞的会说明原因。

输入法能看到你敲的每一个键，所以我们特别在意这些：

- 敲的内容外泄：日志、诊断信息里出现不该出现的输入；密码框里仍在组句。
- 安装包完整性：Release 上的安装包与 `SHA256SUMS`、`releases.json` 里的哈希对不上，或签名有问题。
- 解析崩溃：恶意构造的词库文件（TSV、Rime yaml、`.qj`）或配置文件让输入法崩溃或越界。

不在范围内：需要本机管理员权限或物理接触才能利用的问题。

---

**English.** CloudIME is in beta; only the latest 0.1.x release receives fixes. Please report vulnerabilities privately via
GitHub's "Report a vulnerability" (Security tab), not in public issues. Expect a reply within 3 days; confirmed issues are fixed
in the next release and credited in the changelog. An input method sees every keystroke, so we care most about keystroke leakage
(logs, diagnostics, password fields), installer integrity (installer vs. `SHA256SUMS`), and crashes from
malformed dictionaries or config.
