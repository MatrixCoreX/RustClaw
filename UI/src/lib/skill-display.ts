import type { SkillListItem } from "../types/api";

export type UiLanguage = "zh" | "en";

export interface SkillGroups {
  tool: string[];
  image: string[];
  audio: string[];
  multimedia: string[];
  base: string[];
  other: string[];
}

const UI_HIDDEN_SKILLS = new Set<string>(["chat"]);

/** 基本技能（与后端 base_skill_names 一致），API 未返回时用此兜底 */
const FALLBACK_BASE_SKILL_NAMES = [
  "run_cmd",
  "fs_basic",
  "config_basic",
  "read_file",
  "write_file",
  "list_dir",
  "make_dir",
  "remove_file",
  "schedule",
  "subagent",
  "extension_manager",
  "kb",
  "rss_fetch",
  "system_basic",
  "process_basic",
  "config_guard",
  "fs_search",
  "git_basic",
  "service_control",
  "archive_basic",
];

const SKILL_SUMMARY: Record<string, { zh: string; en: string }> = {
  archive_basic: { zh: "压缩、解压和整理归档文件。", en: "Compress, extract, and organize archives." },
  audio_synthesize: { zh: "把文字转成语音。", en: "Turn text into speech." },
  audio_transcribe: { zh: "把语音转成文字。", en: "Turn speech into text." },
  browser_web: { zh: "打开网页并提取页面内容。", en: "Open webpages and extract page content." },
  config_guard: { zh: "检查配置是否缺项或明显不合理。", en: "Check configs for missing or risky values." },
  config_basic: { zh: "读取并校验结构化配置字段。", en: "Read and validate structured config fields." },
  config_edit: { zh: "预览、修改并校验配置。", en: "Preview, update, and validate configuration." },
  code_index: { zh: "索引并搜索代码结构和符号。", en: "Index and search code structure and symbols." },
  crypto: { zh: "查看币价、账户、订单和交易相关能力。", en: "Handle crypto quotes, balances, orders, and trading tasks." },
  db_basic: { zh: "查看和处理数据库里的基础数据。", en: "Inspect and work with basic database data." },
  doc_parse: { zh: "解析文档内容，提取可读文本。", en: "Parse documents and extract readable text." },
  docker_basic: { zh: "查看和操作 Docker 容器、镜像与服务。", en: "Inspect and control Docker containers, images, and services." },
  extension_manager: { zh: "管理外部扩展技能的接入。", en: "Manage external skill extensions." },
  fs_search: { zh: "在文件里搜索关键词或定位内容。", en: "Search files and locate content." },
  fs_basic: { zh: "处理文件、目录、路径事实和文本搜索。", en: "Handle files, directories, path facts, and text search." },
  git_basic: { zh: "查看提交、分支和常见 Git 操作。", en: "Inspect commits, branches, and common Git actions." },
  health_check: { zh: "快速检查系统和服务是否正常。", en: "Run quick health checks for the system and services." },
  http_basic: { zh: "发起 HTTP 请求并查看返回结果。", en: "Send HTTP requests and inspect responses." },
  image_edit: { zh: "修改、扩图或局部编辑图片。", en: "Edit, extend, or patch images." },
  image_generate: { zh: "根据描述生成图片。", en: "Generate images from prompts." },
  image_vision: { zh: "识别和理解图片内容。", en: "Analyze and understand image content." },
  install_module: { zh: "安装或补齐项目依赖模块。", en: "Install or restore project dependencies." },
  invest_copy: { zh: "整理调研材料并生成投资文案。", en: "Turn research material into investment copy." },
  kb: { zh: "查询和维护本地知识库内容。", en: "Query and maintain local knowledge base content." },
  list_dir: { zh: "查看目录结构和文件列表。", en: "List directories and files." },
  log_analyze: { zh: "分析日志，定位错误和异常。", en: "Analyze logs and find issues." },
  make_dir: { zh: "创建新目录。", en: "Create directories." },
  map_merchant: { zh: "按位置推荐商家或地点。", en: "Recommend nearby merchants or places." },
  music_generate: { zh: "根据描述和歌词生成音乐。", en: "Generate music from prompts and lyrics." },
  package_manager: { zh: "处理包管理、安装与版本问题。", en: "Manage packages, installs, and versions." },
  photo_organize: { zh: "整理照片文件并生成分类建议。", en: "Organize photos and suggest categories." },
  process_basic: { zh: "查看和管理进程。", en: "Inspect and manage processes." },
  read_file: { zh: "读取文件内容。", en: "Read file contents." },
  remove_file: { zh: "删除文件。", en: "Remove files." },
  rss_fetch: { zh: "抓取和整理 RSS 资讯。", en: "Fetch and summarize RSS feeds." },
  run_cmd: { zh: "运行命令行命令。", en: "Run shell commands." },
  schedule: { zh: "创建、查询或管理定时任务。", en: "Create, inspect, or manage scheduled tasks." },
  service_control: { zh: "启动、停止或重启服务。", en: "Start, stop, or restart services." },
  stock: { zh: "股票市场技能。", en: "Stock market skill." },
  subagent: { zh: "把单个或批量只读检查、隔离任务交给受限子代理。", en: "Delegate single or batched read-only reviews and isolated work to bounded child agents." },
  task_control: { zh: "查看、取消当前会话未完成任务。", en: "List and cancel unfinished tasks in the current chat." },
  system_basic: { zh: "查看系统信息和基础环境。", en: "Inspect system information and environment basics." },
  transform: { zh: "转换文本、数据或文件格式。", en: "Transform text, data, or file formats." },
  video_generate: { zh: "根据描述或图片生成视频。", en: "Generate videos from prompts or images." },
  weather: { zh: "查询天气和基础预报信息。", en: "Check weather and basic forecasts." },
  web_search_extract: { zh: "搜索网页并提取关键内容。", en: "Search the web and extract key content." },
  workspace_patch: {
    zh: "用可检查、可回退的补丁修改工作区文件。",
    en: "Modify workspace files with reviewable, reversible patches.",
  },
  write_file: { zh: "写入或修改文件内容。", en: "Write or update file contents." },
  x: { zh: "xurl调用技能。", en: "xurl invocation skill." },
};

const SKILL_USAGE_EXAMPLES: Record<string, { zh: readonly string[]; en: readonly string[] }> = {
  run_cmd: {
    zh: ["检查当前项目的 Git 状态，不要修改文件。", "运行 cargo check 并告诉我是否通过。", "执行 clawcli --help，并概括可用子命令。"],
    en: ["Check this project's Git status without changing files.", "Run cargo check and tell me whether it passes.", "Run clawcli --help and summarize the available subcommands."],
  },
  read_file: {
    zh: ["读取 README.md 的前 40 行并概括。", "打开 configs/config.toml，告诉我当前选择的模型。", "查看 Cargo.toml 里的 workspace members。"],
    en: ["Read the first 40 lines of README.md and summarize them.", "Open configs/config.toml and tell me which model is selected.", "Show me the workspace members in Cargo.toml."],
  },
  write_file: {
    zh: ["新建 notes/todo.md，写入今天的待办事项。", "把这段内容保存到 reports/summary.txt。", "在 CHANGELOG.md 末尾追加这条记录。"],
    en: ["Create notes/todo.md with today's tasks.", "Save this content to reports/summary.txt.", "Append this entry to CHANGELOG.md."],
  },
  workspace_patch: {
    zh: ["修正 src/main.rs 里的拼写错误，并显示补丁。", "预览当前工作区相对检查点的改动。", "撤销刚才的补丁并恢复到对应检查点。"],
    en: ["Fix the typo in src/main.rs and show the patch.", "Preview workspace changes since the checkpoint.", "Rewind the last patch to its checkpoint."],
  },
  list_dir: {
    zh: ["列出当前目录里的文件。", "只列出 configs 目录下的 TOML 文件。", "显示 crates/skills 下的子目录。"],
    en: ["List the files in the current directory.", "List only TOML files under configs.", "Show the subdirectories under crates/skills."],
  },
  make_dir: {
    zh: ["创建 reports 目录。", "新建 data/import/2026，并自动创建父目录。", "在当前工作区建立 notes/archive 文件夹。"],
    en: ["Create a reports directory.", "Create data/import/2026 including parent directories.", "Make a notes/archive folder in this workspace."],
  },
  remove_file: {
    zh: ["删除 tmp/old-report.txt。", "移除刚才生成的测试文件。", "删除 build/obsolete.json，操作前先确认。"],
    en: ["Delete tmp/old-report.txt.", "Remove the test file created in the previous step.", "Delete build/obsolete.json after confirming first."],
  },
  fs_basic: {
    zh: ["检查 README.md 是否存在，并告诉我文件大小。", "在 crates 目录搜索所有 TODO。", "比较本地和 Docker 的技能 registry 是否一致。"],
    en: ["Check whether README.md exists and report its size.", "Search for every TODO under crates.", "Compare the local and Docker skill registries."],
  },
  code_index: {
    zh: ["为当前项目建立代码索引。", "查找 process_ask_task 的定义和引用。", "定位 CapabilityResolver 由哪些模块实现。"],
    en: ["Build a code index for this project.", "Find the definition and references of process_ask_task.", "Locate the modules that implement CapabilityResolver."],
  },
  config_basic: {
    zh: ["读取配置里的 llm.selected_model。", "检查 skills.registry_path 当前是什么。", "同时读取 server.host 和 server.port。"],
    en: ["Read llm.selected_model from the config.", "Check the current skills.registry_path value.", "Read server.host and server.port together."],
  },
  config_edit: {
    zh: ["预览把 server.port 改为 8788，不要写入。", "校验 configs/config.toml 是否有效。", "把 logging.level 改为 info，应用前让我确认。"],
    en: ["Preview changing server.port to 8788 without writing it.", "Validate configs/config.toml.", "Change logging.level to info after asking for confirmation."],
  },
  schedule: {
    zh: ["先解析‘每周一上午九点提醒我开周会’，不要创建任务。", "每天晚上十点提醒我备份文件。", "列出我当前的定时任务。"],
    en: ["Parse 'remind me every Monday at 9 AM' without creating it.", "Remind me to back up files every day at 10 PM.", "List my current scheduled tasks."],
  },
  subagent: {
    zh: ["让只读 review 子代理检查这两个文件是否一致。", "让两个只读子代理分别检查配置和测试，再汇总结果。", "创建一个可恢复的隔离子任务检查这次代码修改。"],
    en: ["Ask a read-only review child agent to compare these two files.", "Have two read-only child agents inspect the config and tests, then aggregate their findings.", "Create a resumable isolated child task to review this code change."],
  },
  x: {
    zh: ["帮我草拟一条产品更新动态，不要发布。", "预览这条 X 动态是否超过长度限制。", "把这条已确认的动态发布到 X。"],
    en: ["Draft a product update post without publishing it.", "Preview whether this X post exceeds the length limit.", "Publish this confirmed post to X."],
  },
  system_basic: {
    zh: ["告诉我当前操作系统和 CPU 架构。", "检查 RustClaw 运行状态。", "统计当前目录有多少文件和子目录。"],
    en: ["Tell me the current operating system and CPU architecture.", "Check the RustClaw runtime status.", "Count files and subdirectories in the current directory."],
  },
  http_basic: {
    zh: ["请求 https://example.com 并告诉我状态码。", "下载这个 URL 到当前工作区。", "向这个测试接口发送一份 JSON 并展示返回结果。"],
    en: ["Request https://example.com and report the status code.", "Download this URL into the current workspace.", "POST this JSON to the test endpoint and show the response."],
  },
  git_basic: {
    zh: ["查看当前 Git 状态。", "显示尚未提交的差异。", "列出最近五次提交。"],
    en: ["Show the current Git status.", "Show the uncommitted diff.", "List the five most recent commits."],
  },
  install_module: {
    zh: ["预览安装 jq，需要哪些命令？", "为这个 Node 项目安装 zod。", "给当前 Rust 项目添加 serde_json 依赖。"],
    en: ["Preview the commands needed to install jq.", "Install zod for this Node project.", "Add the serde_json dependency to this Rust project."],
  },
  process_basic: {
    zh: ["检查 clawd 进程是否正在运行。", "查看 8787 端口由哪个进程监听。", "读取 claw.log 的最后 50 行。"],
    en: ["Check whether the clawd process is running.", "Show which process is listening on port 8787.", "Read the last 50 lines of claw.log."],
  },
  package_manager: {
    zh: ["检测当前系统可用的包管理器。", "判断这个项目使用 npm、pnpm 还是 yarn。", "预览安装 ripgrep，不要实际安装。"],
    en: ["Detect the package managers available on this system.", "Determine whether this project uses npm, pnpm, or yarn.", "Preview installing ripgrep without making changes."],
  },
  archive_basic: {
    zh: ["列出 backup.zip 里的文件。", "读取 archive.tar.gz 里的 README.md。", "把 reports 目录打包成 reports.zip。"],
    en: ["List the files inside backup.zip.", "Read README.md from archive.tar.gz.", "Pack the reports directory as reports.zip."],
  },
  db_basic: {
    zh: ["列出 data/app.db 里的所有表。", "查询 users 表前十条记录，不要修改数据库。", "告诉我这个 SQLite 数据库的 schema version。"],
    en: ["List all tables in data/app.db.", "Query the first ten rows from users without changing the database.", "Report this SQLite database's schema version."],
  },
  docker_basic: {
    zh: ["列出正在运行的 Docker 容器。", "查看 api 容器最近 100 行日志。", "显示本机 Docker 版本。"],
    en: ["List running Docker containers.", "Show the latest 100 log lines from the api container.", "Show the installed Docker version."],
  },
  fs_search: {
    zh: ["在 src 目录搜索 error_kind。", "查找项目里所有名为 AGENTS.md 的文件。", "搜索包含 TODO 的 Rust 文件。"],
    en: ["Search for error_kind under src.", "Find every file named AGENTS.md in the project.", "Search Rust files containing TODO."],
  },
  rss_fetch: {
    zh: ["读取这个 RSS 地址的最新五条内容。", "抓取该订阅并按发布时间排序。", "提取这个 Atom feed 的标题和链接。"],
    en: ["Fetch the five latest items from this RSS URL.", "Fetch this feed and sort items by publication time.", "Extract titles and links from this Atom feed."],
  },
  image_vision: {
    zh: ["描述这张图片里有什么。", "提取截图中的文字。", "比较这两张图片有哪些不同。"],
    en: ["Describe what is shown in this image.", "Extract the text from this screenshot.", "Compare these two images and identify the differences."],
  },
  image_generate: {
    zh: ["生成一张雨后竹林的写实图片。", "画一张适合博客封面的咖啡店插图。", "先预览生成参数，不要调用图片 API。"],
    en: ["Generate a realistic image of a bamboo forest after rain.", "Create a coffee-shop illustration for a blog cover.", "Preview the generation parameters without calling the image API."],
  },
  image_edit: {
    zh: ["把这张图片的背景改成纯白。", "移除照片右下角的杂物。", "把这张横图扩展成 16:9。"],
    en: ["Change this image's background to pure white.", "Remove the clutter in the bottom-right corner.", "Extend this landscape image to 16:9."],
  },
  audio_transcribe: {
    zh: ["把这段录音转成文字。", "转写 meeting.m4a，并标出时间戳。", "识别这段中文语音的内容。"],
    en: ["Transcribe this recording into text.", "Transcribe meeting.m4a with timestamps.", "Recognize the spoken content in this audio file."],
  },
  audio_synthesize: {
    zh: ["把这段文字生成普通话语音。", "用自然的英文声音朗读这段内容。", "先预览语音合成参数，不要生成文件。"],
    en: ["Turn this text into Mandarin speech.", "Read this content with a natural English voice.", "Preview the speech synthesis parameters without creating a file."],
  },
  video_generate: {
    zh: ["生成一段海边日落的五秒视频。", "让这张产品图生成缓慢旋转的视频。", "先做视频生成 dry run，不要调用提供商。"],
    en: ["Generate a five-second video of a beach sunset.", "Animate this product image with a slow rotation.", "Dry-run the video generation without calling the provider."],
  },
  music_generate: {
    zh: ["生成一段轻快的无歌词背景音乐。", "根据这段歌词创作一首流行歌曲。", "先预览音乐生成任务，不要调用提供商。"],
    en: ["Generate upbeat instrumental background music.", "Create a pop song from these lyrics.", "Preview the music generation job without calling the provider."],
  },
  health_check: {
    zh: ["检查 RustClaw 当前是否健康。", "做一次基础服务健康检查。", "告诉我哪些依赖没有正常工作。"],
    en: ["Check whether RustClaw is healthy.", "Run a basic service health check.", "Tell me which dependencies are not working."],
  },
  log_analyze: {
    zh: ["分析 claw.log 最近的错误。", "找出这份构建日志失败的根因。", "统计日志里最常见的三类异常。"],
    en: ["Analyze the recent errors in claw.log.", "Find the root cause of this build log failure.", "Count the three most common error types in this log."],
  },
  service_control: {
    zh: ["查看 clawd 服务状态。", "重启 nni-server 服务。", "停止这个服务，操作前先确认。"],
    en: ["Show the clawd service status.", "Restart the nni-server service.", "Stop this service after asking for confirmation."],
  },
  task_control: {
    zh: ["列出当前对话里未完成的任务。", "预览恢复这个任务需要哪些字段，不要实际恢复。", "取消第二个排队任务。"],
    en: ["List unfinished tasks in this conversation.", "Preview the fields needed to resume this task without resuming it.", "Cancel the second queued task."],
  },
  config_guard: {
    zh: ["检查主配置有没有缺少必填项。", "找出配置里风险较高的设置。", "检查技能 registry 的配置是否完整。"],
    en: ["Check whether the main config is missing required values.", "Find potentially risky settings in the config.", "Check whether the skill registry configuration is complete."],
  },
  crypto: {
    zh: ["查询 BTC 当前价格。", "比较 BTC 在 Binance 和 OKX 的报价。", "预览买入 20 USDT 的 ETH，不要下单。"],
    en: ["Check the current BTC price.", "Compare BTC quotes on Binance and OKX.", "Preview buying 20 USDT of ETH without placing an order."],
  },
  stock: {
    zh: ["查询贵州茅台当前行情。", "告诉我 600519 的最新价格。", "查看这只 A 股今天的涨跌幅。"],
    en: ["Check the current quote for Kweichow Moutai.", "Tell me the latest price of 600519.", "Show today's percentage change for this A-share stock."],
  },
  weather: {
    zh: ["查询上海今天的天气。", "告诉我北京未来三天会不会下雨。", "查看深圳明天的最高和最低温度。"],
    en: ["Check today's weather in Shanghai.", "Tell me whether it will rain in Beijing over the next three days.", "Show tomorrow's high and low temperatures in Shenzhen."],
  },
  map_merchant: {
    zh: ["推荐我附近三家评分高的川菜馆。", "在上海虹桥站附近找一家安静的咖啡店。", "按距离推荐五家人均 100 元以内的餐厅。"],
    en: ["Recommend three highly rated Sichuan restaurants nearby.", "Find a quiet coffee shop near Shanghai Hongqiao Station.", "Recommend five nearby restaurants under 100 CNY per person."],
  },
  doc_parse: {
    zh: ["提取这份 PDF 的关键要点。", "总结合同里的付款和违约条款。", "读取这个文档的目录与主要章节。"],
    en: ["Extract the key points from this PDF.", "Summarize the payment and breach clauses in this contract.", "Read this document's table of contents and main sections."],
  },
  transform: {
    zh: ["把这组 JSON 按 price 从高到低排序。", "将这段 CSV 去重后输出 Markdown 表格。", "按 category 分组并汇总 amount。"],
    en: ["Sort this JSON array by price in descending order.", "Deduplicate this CSV and output a Markdown table.", "Group by category and sum amount."],
  },
  invest_copy: {
    zh: ["根据这些数据写一份巴菲特风格的投资解读。", "列出可用的投资人物风格。", "把这份研究数据整理成含风险提示的投资文案。"],
    en: ["Write a Buffett-style investment analysis from this data.", "List the available investor personas.", "Turn this research data into investment copy with risk disclosures."],
  },
  web_search_extract: {
    zh: ["搜索 Rust 1.90 的官方发布说明并提取重点。", "查找这个问题的三个可靠来源。", "搜索最新资料并给出标题、链接和摘要。"],
    en: ["Search for the official Rust 1.90 release notes and extract the highlights.", "Find three reliable sources about this issue.", "Search for current information and return titles, links, and summaries."],
  },
  kb: {
    zh: ["把 docs 目录加入 project_docs 知识库。", "在 project_docs 里搜索部署流程。", "列出当前已有的知识库。"],
    en: ["Add the docs directory to the project_docs knowledge base.", "Search project_docs for the deployment process.", "List the available knowledge bases."],
  },
  browser_web: {
    zh: ["打开这个网页并提取正文。", "读取该页面标题并总结主要内容。", "打开这个链接，找出页面里的下载地址。"],
    en: ["Open this webpage and extract the main text.", "Read the page title and summarize the main content.", "Open this link and find the download URL on the page."],
  },
  photo_organize: {
    zh: ["预览如何整理移动硬盘里的照片，不要移动文件。", "按拍摄日期规划整理这个照片目录。", "把这些照片按相机型号复制到分类目录。"],
    en: ["Preview how to organize photos on the external drive without moving files.", "Plan how to organize this photo directory by capture date.", "Copy these photos into folders grouped by camera model."],
  },
  extension_manager: {
    zh: ["评估这个需求应该临时处理还是开发成新技能。", "为天气预警能力创建一个外部技能脚手架。", "验证这个外部技能能否注册，暂时不要启用。"],
    en: ["Assess whether this request needs a temporary fix or a reusable skill.", "Scaffold an external skill for weather alerts.", "Validate whether this external skill can be registered without enabling it yet."],
  },
};

function copy(lang: UiLanguage, zh: string, en: string): string {
  return lang === "zh" ? zh : en;
}

function sortSkillNames(names: string[]): string[] {
  return [...names].sort((a, b) => a.localeCompare(b));
}

export function isUiHiddenSkill(name?: string | null): boolean {
  return Boolean(name && UI_HIDDEN_SKILLS.has(name));
}

export function isVisibleSkillName(name?: string | null): name is string {
  return Boolean(name && !isUiHiddenSkill(name));
}

export function visibleSkillNames(names?: string[] | null): string[] {
  return (names ?? []).filter(isVisibleSkillName);
}

export function baseSkillNamesWithFallback(names?: string[] | null): string[] {
  const source = names && names.length > 0 ? names : FALLBACK_BASE_SKILL_NAMES;
  return visibleSkillNames(source);
}

export function normalizeSkillSearchQuery(query: string): string {
  return query.trim().toLowerCase();
}

export function filterSkillNamesBySearch(names: string[], normalizedQuery: string): string[] {
  if (!normalizedQuery) return names;
  return names.filter((name) => name.toLowerCase().includes(normalizedQuery));
}

export function groupSkillNames(
  managedSkills: string[],
  baseSkillNamesSet: ReadonlySet<string>,
  toolSkillNamesSet: ReadonlySet<string>,
): SkillGroups {
  const isMultimedia = (name: string) => name.startsWith("video_") || name.startsWith("music_");
  return {
    tool: sortSkillNames(managedSkills.filter((name) => toolSkillNamesSet.has(name))),
    image: sortSkillNames(managedSkills.filter((name) => name.startsWith("image_") && !toolSkillNamesSet.has(name))),
    audio: sortSkillNames(managedSkills.filter((name) => name.startsWith("audio_") && !toolSkillNamesSet.has(name))),
    multimedia: sortSkillNames(managedSkills.filter((name) => isMultimedia(name) && !toolSkillNamesSet.has(name))),
    base: sortSkillNames(managedSkills.filter((name) => baseSkillNamesSet.has(name) && !toolSkillNamesSet.has(name))),
    other: sortSkillNames(
      managedSkills.filter(
        (name) =>
          !name.startsWith("image_") &&
          !name.startsWith("audio_") &&
          !isMultimedia(name) &&
          !baseSkillNamesSet.has(name) &&
          !toolSkillNamesSet.has(name),
      ),
    ),
  };
}

export function skillDescription(name: string, lang: UiLanguage, itemDescription?: string | null): string {
  const summary = SKILL_SUMMARY[name];
  if (summary) return copy(lang, summary.zh, summary.en);
  const description = itemDescription?.trim();
  if (description) return description;
  return copy(lang, "该技能无简短说明。", "No short description for this skill.");
}

export function hasCuratedSkillUsageExamples(name: string): boolean {
  return Boolean(SKILL_USAGE_EXAMPLES[name]);
}

export function skillUsageExamples(
  name: string,
  lang: UiLanguage,
  itemDescription?: string | null,
): readonly string[] {
  const examples = SKILL_USAGE_EXAMPLES[name];
  if (examples) return examples[lang];
  const description = itemDescription?.trim() || name;
  return lang === "zh"
    ? [
        `帮我处理这项需求：${description}`,
        `先检查能否完成这项任务，不要执行有副作用的操作：${description}`,
        `完成这项任务，并告诉我结果和下一步：${description}`,
      ]
    : [
        `Help me with this request: ${description}`,
        `Check whether this can be completed without performing side effects: ${description}`,
        `Complete this task and tell me the result and next step: ${description}`,
      ];
}

export function skillRiskLabel(risk: string | null | undefined, lang: UiLanguage): string {
  switch ((risk || "").toLowerCase()) {
    case "low":
      return copy(lang, "低风险", "Low risk");
    case "medium":
      return copy(lang, "中风险", "Medium risk");
    case "high":
      return copy(lang, "高风险", "High risk");
    default:
      return copy(lang, "风险未声明", "Risk not declared");
  }
}

export function skillCapabilityLabel(capability: string, lang: UiLanguage): string {
  switch (capability) {
    case "llm":
      return copy(lang, "会调用模型", "Uses model");
    case "net":
      return copy(lang, "访问网络", "Network");
    case "fs.read":
      return copy(lang, "读取文件", "Reads files");
    case "fs.write":
      return copy(lang, "改写文件", "Changes files");
    case "exec":
      return copy(lang, "运行命令", "Runs commands");
    case "exec.sudo":
      return copy(lang, "可提权执行", "Can use sudo");
    default:
      return capability.startsWith("secrets.") ? copy(lang, "需要密钥", "Needs secret") : capability;
  }
}

export function formatCapabilityToken(token: string): string {
  return token
    .split(".")
    .map((part) => part.replace(/_/g, " "))
    .join(" / ");
}

export function skillPlannerCapabilityLabel(capability: string, lang: UiLanguage): string {
  const [domain, ...rest] = capability.split(".");
  const readable = formatCapabilityToken(rest.join(".") || capability);
  const domainLabel = {
    filesystem: copy(lang, "文件", "Files"),
    config: copy(lang, "配置", "Config"),
    system: copy(lang, "系统", "System"),
    database: copy(lang, "数据库", "Database"),
  }[domain];
  return domainLabel ? `${domainLabel}: ${readable}` : formatCapabilityToken(capability);
}

export function skillIsolationLabels(item: SkillListItem | undefined, lang: UiLanguage): string[] {
  const policies = item?.planner_capability_policies ?? [];
  const labels: string[] = [];
  const push = (label: string) => {
    if (!labels.includes(label)) labels.push(label);
  };
  for (const policy of policies) {
    switch (policy.isolation_profile) {
      case "read_only":
        push(copy(lang, "只读", "Read-only"));
        break;
      case "local_current_workspace":
        push(copy(lang, "当前工作区", "Current workspace"));
        break;
      case "local_worktree":
        push(copy(lang, "独立工作树", "Separate worktree"));
        break;
      case "local_temp_workspace":
        push(copy(lang, "临时工作区", "Temp workspace"));
        break;
      case "remote_executor":
        push(copy(lang, "外部执行", "External execution"));
        break;
    }
    if (policy.network_access) push(copy(lang, "访问网络", "Network"));
    if (policy.filesystem_write) push(copy(lang, "可改文件", "Can edit files"));
    if (policy.external_publish) push(copy(lang, "可对外发布", "Can publish"));
    if (policy.credential_access) push(copy(lang, "使用密钥", "Uses keys"));
    if (policy.subprocess) push(copy(lang, "运行子进程", "Runs subprocesses"));
    if (policy.package_install) push(copy(lang, "安装软件包", "Installs packages"));
    if (policy.privilege_escalation) push(copy(lang, "可能提权", "May elevate privileges"));
  }
  return labels;
}

export function skillRuntimeIssue(item: SkillListItem | undefined, lang: UiLanguage): string | null {
  if (!item || item.runtime_available !== false) return null;
  if (item.unavailable_reason === "skill_disabled" || item.enabled === false) {
    return copy(lang, "该技能当前未开启", "This skill is currently disabled");
  }
  if (item.unsupported_os?.length) {
    return copy(
      lang,
      `当前系统 ${item.current_os || "unknown"} 不在支持列表：${item.unsupported_os.join(", ")}`,
      `Current OS ${item.current_os || "unknown"} is not supported: ${item.unsupported_os.join(", ")}`,
    );
  }
  if (item.missing_required_bins?.length) {
    return copy(
      lang,
      `缺少本地工具：${item.missing_required_bins.join(", ")}`,
      `Missing local tools: ${item.missing_required_bins.join(", ")}`,
    );
  }
  return copy(lang, "当前设备暂不可用", "Unavailable on this device");
}
