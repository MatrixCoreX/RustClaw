# RustClaw 技能前置条件与绑定方式

这份文档给 RustClaw 回答“某个 skill 怎么开通 / 怎么绑定 / 怎么配置 / 缺什么前置条件”时使用。目标不是背固定话术，而是让模型知道当前仓库里真实的配置入口、作用域和下一步操作方式。

## 回答原则

1. 先说真实入口，再说操作建议。优先说明它到底是改配置文件、写环境变量、走本地数据库、调用已有 API，还是需要本机依赖 / 第三方登录。
2. 明确作用域。要分清这是“当前机器全局配置”“当前仓库配置”“按 `user_key` 存储的用户级绑定”，还是“当前会话 / 当前通道登录态”。
3. 如果只差一个关键信息，就顺手追问。不要只停在说明层。若缺的是普通参数或路径，可以直接追问；若缺的是 secret / token / password / API key，优先引导用户走专用命令、UI、本地配置或 API，不要让用户把敏感值发到普通对话。
4. 不要先把能自动做的事推回给用户手改。如果 RustClaw 已经有 `read_file` / `write_file` / `run_cmd` / 现成 API，就优先让模型知道可以继续调用底层能力落地修改。
5. 不要把 `configs/*.toml` 和数据库型绑定混为一谈。尤其是 `crypto`：交易所凭据正常路径是按 `user_key` 写入本地数据库，不是让用户直接改 `configs/crypto.toml`。
6. `configs/` 下的配置文件修改是 admin 权限能力。当前任务不是 admin 时，应该直接回复没有权限，不要继续尝试修改。

## 技能总表

| 技能 | 真实前置条件 / 配置入口 | 回答时应强调什么，下一步该问什么 |
| --- | --- | --- |
| `run_cmd` `read_file` `write_file` `list_dir` `make_dir` `remove_file` | 无额外绑定；直接依赖当前工作区和本机权限 | 这些是本地基础能力，不需要单独开通。若用户要改文件、写文件、创建目录或执行命令，直接继续执行即可。 |
| `schedule` | 无第三方绑定；依赖 RustClaw 调度能力 | 如果用户问怎么用，说明需要任务内容和时间表达式；下一步追问“你要我帮你建什么定时任务、什么时候触发”。 |
| `system_basic` `process_basic` `health_check` `log_analyze` `service_control` `task_control` `config_guard` | 无第三方绑定；直接依赖本机环境、服务和配置文件 | 这些属于本地运维 / 检查能力。通常不需要额外配置；如果失败，多半是权限、目标服务不存在，或路径不对。 |
| `archive_basic` `fs_search` `git_basic` `package_manager` `install_module` `docker_basic` `db_basic` `http_basic` `doc_parse` `transform` | 主要依赖本机命令、文件和网络；`git_basic` 读 `configs/git_basic.toml`；`db_basic` 默认库是 `data/rustclaw.db` | 这类 skill 通常不用“绑定账号”。如果用户要改默认行为，再去改对应配置或命令参数。`db_basic` 这类要追问具体库路径 / SQL / 目标表。 |
| `rss_fetch` | 读 `configs/rss.toml`；源列表、分类、失败退避都在这里 | 告诉用户 RSS 不是账号绑定型 skill，重点是 feed 源配置。若要补源，追问“你要加到哪个 category，feed URL 是什么”。 |
| `browser_web` | 依赖 `crates/skills/browser_web/browser_web.js`、Node.js、Playwright；等待策略在 `configs/browser_web_wait_map.json` | 如果用户问为什么不能用，优先说明是本机浏览器依赖型 skill。下一步追问“要不要我先检查 / 安装 Node.js 和 Playwright，或帮你调 wait map”。 |
| `web_search_extract` | 搜索后端来自 `args.backend` / `WEB_SEARCH_BACKEND`；`serpapi` 需要 `SERPAPI_API_KEY`；DuckDuckGo HTML 路径可零 key 运行 | 说明它是“搜索后端 + 抽取”型 skill，不是仓库配置文件绑定为主。若用户要更稳的搜索，追问“你要不要配 `SERPAPI_API_KEY`，或者先继续走 DuckDuckGo fallback”。 |
| `image_generate` `image_edit` `image_vision` | 主要读 `configs/image.toml`；也会继承 `configs/config.toml` 里的 `[llm.*]`；支持环境变量覆盖，如 `OPENAI_API_KEY`、`QWEN_API_KEY`、`MINIMAX_API_KEY`，以及 `IMAGE_GENERATION_*` / `IMAGE_EDIT_*` / `IMAGE_VISION_*` | 回答时要先说当前用哪个 vendor / model，再说 key 放在哪里。下一步追问“你要走哪个 provider，我现在帮你把 key 写进配置还是走环境变量”。 |
| `audio_transcribe` `audio_synthesize` | 主要读 `configs/audio.toml`；也会继承 `configs/config.toml` 的 `[llm.*]`；支持 `OPENAI_API_KEY`、`QWEN_API_KEY`、`MINIMAX_API_KEY` 等，以及 `AUDIO_TRANSCRIBE_*` / `AUDIO_SYNTHESIZE_*` 覆盖 | 这是 provider-key 型 skill。回答时要说明 STT / TTS 可以单独覆写，也可以直接沿用全局 `[llm]`。下一步追问“你要我帮你配哪家 provider 的 key”。 |
| `crypto` | 交易策略 / 白名单在 `configs/crypto.toml`；交易所凭据按 `user_key` 存本地数据库表 `exchange_api_credentials`；可通过 Telegram `/cryptoapi set ...` 或 `POST /v1/auth/crypto-credentials` 绑定 | 这是最容易答错的地方。要明确说：`configs/crypto.toml` 主要是策略与限制，Binance / OKX 的用户凭据正常路径是写数据库，不是让用户手改 TOML。`/cryptoapi set ...` 和 `POST /v1/auth/crypto-credentials` 都是“当前 key 新增或覆盖自己在该交易所的凭据”，不是替别人修改。对交易所范围明确的操作，如果用户没写交易所，先看 `crypto.execution_mode` / `crypto.default_exchange`；有默认值就按默认值，没有默认值才反问。优先引导用户用 Telegram `/cryptoapi set ...`，因为这条命令在 `telegramd` 侧直接处理，不走普通 `ask` 推理流。若用户需要继续指导，只补命令格式或缺的非敏感字段，不要让用户把 raw secret 发到普通对话。 |
| `stock` | 主配置在 `configs/stock.toml`；股票名映射在 `[stock.aliases]`；名称纠错可借助 `configs/config.toml` 中已配置的 `[llm.*]` provider | 这不是账号绑定型 skill。若用户问“怎么配”，重点应放在别名映射和名称纠错。下一步追问“你要查具体代码，还是要我把这个公司名补进别名表”。 |
| `weather` | 主配置在 `configs/weather.toml`；主要是语言 / i18n 路径 | 无账号绑定。若用户问“怎么开通”，应告诉他通常零配置即可用；如果要改输出语言，再改 `configs/weather.toml`。 |
| `map_merchant` | 主配置在 `configs/map_merchant.toml`；Amap 可走 `AMAP_API_KEY` 或配置文件；Google 可走 `GOOGLE_MAPS_API_KEY` / `GOOGLE_PLACES_API_KEY` 或配置文件 | 这是地图 provider key 型 skill。回答时要先说明当前默认 provider，再说 key 可以放 env 或 TOML。下一步追问“你要用高德还是 Google，要不要我现在把 key 配进去”。 |
| `kb` | 无第三方账号绑定；知识库存储在工作区 `data/kb/by_user/...`，统一索引会同步进当前数据库；运行时依赖 `workspace_root` 和待 ingest 的路径 | 重点不是“绑 API”，而是“先 ingest 哪些文件 / 目录”。下一步追问“你要建哪个 namespace，要导入哪些路径”。 |
| `x` | 主配置在 `configs/x.toml`；也可用 `X_USE_XURL`、`XURL_BIN`、`XURL_APP`、`XURL_AUTH`、`XURL_USERNAME` 等环境变量覆盖；真正发帖依赖本机 `xurl` OAuth 登录态 | 这是本机登录态 skill，不是只填 key 就行。回答时要说明要先准备好 `xurl auth oauth2` 的授权。下一步追问“你要我先检查 `xurl` 和当前登录态，还是你要把相关配置发来让我代配”。 |
| `photo_organize` | 主配置在 `configs/photo_organize.toml`；不需要第三方账号；真正需要的是可访问的 `source_dir` | 这类 skill 不需要“绑定”，需要的是明确目录。下一步追问“照片目录的绝对路径是什么，要先 preview 还是直接 copy / move”。 |
| `extension_manager` | 依赖 `OPENAI_API_KEY`；会读写 `external_skills/`、`configs/skills_registry.toml`、`configs/config.toml` | 这是开发型 skill。回答时要说明它会生成 / 注册 / 启用 external skill，而不是普通用户配置项。下一步追问“你要扩什么能力、skill 名叫什么、要不要我先 scaffold 再注册”。 |

## 典型回答模板

### 1. 纯解释型

“`crypto` 的交易所凭据不是改 `configs/crypto.toml`，而是按当前 `user_key` 写进本地数据库的 `exchange_api_credentials`。`configs/crypto.toml` 主要管允许的交易所、交易对和风控限制。你可以走 Telegram 的 `/cryptoapi set ...`，也可以走 `POST /v1/auth/crypto-credentials`；这两条路径都是给当前 key 新增或覆盖自己在该交易所的凭据。” 

### 2. 解释 + 顺手接下一步

“`crypto` 的 Binance / OKX 凭据会按当前 key 写进本地数据库，不是直接改 TOML。为了避免把密钥发到普通对话，建议直接在 Telegram 用 `/cryptoapi set ...` 绑定。如果你不确定格式，我可以把 Binance 或 OKX 对应的完整命令写给你。”

“如果你是要修改交易所 API key，不是改全局配置文件，而是更新当前 key 在该交易所下的那条凭据记录。最直接的做法就是重新执行一次 `/cryptoapi set ...`，它会覆盖你自己当前 key 的旧值。” 

“如果你只说‘帮我修改交易所 API key’，但没说是 Binance 还是 OKX，就先看 `crypto.execution_mode` / `crypto.default_exchange`。如果当前已经配置了默认交易所，就直接按默认值给对应的 `/cryptoapi set ...` 命令；只有默认值也没配置时，才反问一次‘你要改哪个交易所，Binance 还是 OKX？’。” 

### 3. 配置文件型

“`map_merchant` 的 key 可以放环境变量，也可以写进 `configs/map_merchant.toml`。你先告诉我用高德还是 Google；如果要我继续配，我会告诉你应该写哪个配置位，敏感 key 不要直接发到普通对话。”

## 对 `crypto` 的特别约束

- 不要再把“未绑定交易所 API”默认解释成“去手改 `configs/crypto.toml` 的密钥字段”。
- 要明确区分：
  - `configs/crypto.toml`：策略、白名单、默认交易所、限额等全局配置。
  - `exchange_api_credentials`：当前 `user_key` 的交易所凭据，本地数据库持久化。
- 如果用户只是问“怎么绑”，先解释真实路径；如果用户已经表达出想继续绑定，就顺手追问所缺参数，而不是停在教程层。
