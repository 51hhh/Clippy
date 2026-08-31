# 贡献指南

开发环境搭建、常用命令与架构概览见 [CLAUDE.md](CLAUDE.md) 与 [docs/architecture.md](docs/architecture.md)。
提交前跑一遍完整本地门禁：

```bash
./scripts/ci-local.sh
```

约定：代码注释、commit message、文档用**中文**；前端 UI 文本用**英文**（多语言走
`src/i18n/`）。

## 一个 json 就是一条有效 PR：截图几何

截图这块最难缠的一类 bug 是"在我这台机器上是对的"。显示器的位置、尺寸、缩放、旋转、
镜像组合起来的环境几乎无穷多，而**几何算错时代码路径完全不报错**——它只是算出一组
错的数字，用户看到的是覆盖层错位、比屏幕大一圈、画面溢到隔壁屏。这种环境没法靠"挨个
买显示器来测"覆盖。

所以这里把环境本身做成了数据：`src-tauri/tests/fixtures/monitor-layouts/*.json`，
一个文件就是一种显示器配置 + 一条回归测试。切分逻辑（`plan_stage_split`）是纯函数，
不需要真实屏幕、不碰一个像素，所以你的环境可以在 CI 上跑。

**如果你遇到了几何问题，最有价值的贡献就是把你那套环境变成一个 json 文件：**

```bash
# 只吐那段 json，直接重定向进 fixture 目录
clippy --emit-test-case > src-tauri/tests/fixtures/monitor-layouts/我的环境.json

# 想先看完整报告（人读版，含逐来源几何与不变量自检结果）
clippy --capture-diagnose
```

设置 → Screenshot → **Run Diagnostics** 是同一份东西的图形入口，报告末尾的
`monitor-layout` 段落就是这个 json。

然后**只需改两个字段**：

- `name` —— 一句能认出这套环境的话，例如 "GNOME 双屏混合缩放（主屏在右下偏移）"。
- `note` —— 这套配置是怎么来的、当时的症状是什么。写给下一个人看，是这个文件里最值钱
  的部分。已经修好的 bug 也要写进去：`gnome-dual-mixed-scale.json` 的 note 记的就是
  "曾经拿每块屏自己的缩放去除舞台裁剪宽度"，那句话本身就是这条测试存在的理由。

`expect` 段落不用改，也**不要**手填——它是诊断跑真实管线算出来的。手填的期望值只能
证明填的人当时怎么想；这里要钉住的是"这套配置下管线的行为"。

**已知有问题的配置照样欢迎提。** 把症状钉住比把它挡在门外有用：`expect.invariants`
里带着失败标签的 fixture 也会被接受，它记录的是"这个环境目前会这样错"，修好的时候
那条测试自然会红，提醒你把期望值一起更新。

报告不含任何截图像素，也不含任何窗口标题（标题会泄露你正在做什么），只写在本机缓存
目录，绝不自动上传。不确定要不要公开的话，先自己读一遍——它就是一段纯文本。

不想提 PR 也可以直接开 issue：[截图几何错位模板](.github/ISSUE_TEMPLATE/capture-geometry.yml)
会引导你贴上这份报告，剩下的我们来做。

几何这一层的完整设计（坐标空间判定、不变量 I1–I5、为什么所有自检都只记日志不中断截图）
见 [docs/capture-linux.md](docs/capture-linux.md) §4。
