import { useEffect, useRef, useState } from "react";
import { Bot, Check, Copy } from "lucide-react";
import { writeTextToClipboard } from "../lib/clipboard";
import type { AippCatalogItem } from "../types/api";

type Translate = (zh: string, en: string) => string;

function Example({ label, text, t }: { label: string; text: string; t: Translate }) {
  const [copied, setCopied] = useState(false);
  const [failed, setFailed] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; if (timer.current) clearTimeout(timer.current); }; }, []);
  useEffect(() => { setCopied(false); setFailed(false); if (timer.current) clearTimeout(timer.current); }, [text]);
  const copy = async () => {
    try {
      await writeTextToClipboard(text);
      if (!mounted.current) return;
      setCopied(true); setFailed(false);
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => setCopied(false), 2000);
    } catch { if (mounted.current) setFailed(true); }
  };
  return <div className="min-w-0 border-l-2 border-[var(--theme-border)] pl-3">
    <div className="mb-1 flex items-center gap-2">
      <h3 className="min-w-0 flex-1 text-sm font-medium">{label}</h3>
      <button type="button" className="theme-icon-btn h-8 w-8 shrink-0" onClick={() => void copy()} title={copied ? t("已复制", "Copied") : t("复制示例", "Copy example")} aria-label={copied ? t("已复制", "Copied") : t(`复制：${label}`, `Copy: ${label}`)}>
        {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
      </button>
    </div>
    <p className="select-text whitespace-pre-wrap break-words text-sm leading-6 text-[var(--theme-text-body)] [overflow-wrap:anywhere]">{text}</p>
    {failed ? <p role="alert" className="mt-1 text-xs">{t("复制失败，请选中文字手动复制。", "Copy failed. Select the text and copy it manually.")}</p> : null}
  </div>;
}

export function AippUsageGuide({ app, title, description, platforms, t, onOpenAgent }: {
  app: AippCatalogItem; title: string; description: string; platforms: string[];
  t: Translate; onOpenAgent: () => void;
}) {
  const collection = app.renderer === "collection_feed_v1" && app.data_contract === "media_collection_v1";
  const activity = app.renderer === "task_activity_v1" && app.data_contract === "skill_task_activity_v1";
  const [platformInput, setPlatformInput] = useState("");
  const [keywordInput, setKeywordInput] = useState("");
  const [linkInput, setLinkInput] = useState("");
  const platform = platformInput.trim() || t("【平台名称】", "[platform name]");
  const keyword = keywordInput.trim() || t("【搜索关键词】", "[search keywords]");
  const link = linkInput.trim() || t("【媒体链接】", "[media URL]");
  const fieldClass = "theme-input mt-1 w-full min-w-0 px-3 py-2 text-sm";
  const sectionClass = "space-y-3 border-t border-[var(--theme-border)] py-5";
  return <div className="mx-auto min-w-0 max-w-4xl text-[var(--theme-text-strong)] [overflow-wrap:anywhere]" data-testid="aipp-usage-guide">
    <header className="pb-4">
      <h2 className="text-lg font-semibold">{t(`${title}使用说明`, `Using ${title}`)}</h2>
      <p className="mt-2 text-sm leading-6 text-[var(--theme-text-body)]">{t("AiAPP 用来查看任务保存的结果，不会因为打开页面就开始执行。先在 Agent 对话中提出请求，等任务实际产生数据后，再回到「查看结果」。", "AiAPP displays results saved by tasks; opening this page does not start work. Send a request in Agent first, then return to Results after the task produces data.")}</p>
      {description ? <p className="mt-2 text-sm leading-6 text-[var(--theme-text-muted)]">{description}</p> : null}
      <button type="button" className="theme-secondary-btn mt-3 px-3 py-2 text-sm" onClick={onOpenAgent}><Bot className="h-4 w-4" />{t("打开 Agent", "Open Agent")}</button>
    </header>
    <section className={sectionClass}>
      <h2 className="text-base font-semibold">{t("1. 开始前准备", "1. Before you start")}</h2>
      <ol className="list-decimal space-y-2 pl-5 text-sm leading-6 text-[var(--theme-text-body)]">
        <li>{t("先配置可用的大模型，并确认对应技能已经安装、启用；只有安装 AiAPP 还不够。", "Configure a working model and install and enable the associated skill. Installing AiAPP alone is not enough.")}</li>
        <li>{t("保持设备在线。需要平台登录、浏览器验证或访问授权时，按 Agent 提示完成，不要反复发送同一启动指令。", "Keep the device online. Complete platform login, browser verification or access approval when Agent asks; do not repeatedly submit the same start request.")}</li>
        <li>{t("示例中的括号内容需要替换。先说明平台或链接、要做什么、数量与停止条件；需要授权时再确认。", "Replace bracketed placeholders. Specify the platform or URL, the action, quantity and stopping condition, then confirm any required authorization.")}</li>
      </ol>
    </section>
    <section className={sectionClass}>
      <h2 className="text-base font-semibold">{t("2. 在 Agent 中启动", "2. Start in Agent")}</h2>
      {collection ? <>
        <div className="grid min-w-0 gap-3 sm:grid-cols-2">
          <label className="min-w-0 text-sm">{t("要采集的平台", "Platform to collect from")}<input className={fieldClass} value={platformInput} onChange={event => setPlatformInput(event.target.value)} maxLength={80} placeholder={t("填写平台名称", "Enter a platform name")} /></label>
          <label className="min-w-0 text-sm">{t("搜索关键词（可选）", "Search keywords (optional)")}<input className={fieldClass} value={keywordInput} onChange={event => setKeywordInput(event.target.value)} maxLength={200} placeholder={t("例如：财经、旅行、美食", "For example: finance, travel, food")} /></label>
        </div>
        {platforms.length ? <p className="text-xs text-[var(--theme-text-muted)]">{t("当前记录中的平台：", "Platforms in current records: ")}{platforms.join(" / ")}</p> : null}
        <Example t={t} label={t("先确认能采集哪些平台", "Check supported platforms first")} text={t(`请检查「${title}」当前支持的平台和运行条件，告诉我哪些已经可以使用，先不要开始采集。`, `Check the platforms and prerequisites supported by ${title}. Tell me what is ready, without starting collection.`)} />
        <Example t={t} label={t("先少量试采集", "Try a small collection first")} text={t(`请用「${title}」采集${platform}推荐内容，先采集 5 条，保存结果后结束。`, `Use ${title} to collect 5 recommended posts from ${platform}, save the results, then stop.`)} />
        <Example t={t} label={t("按关键词搜索后采集", "Search by keyword and collect")} text={t(`请用「${title}」在${platform}搜索「${keyword}」，按搜索结果顺序采集 30 条，然后停止。`, `Use ${title} to search ${platform} for "${keyword}". Collect 30 posts in search-result order, then stop.`)} />
        <Example t={t} label={t("持续后台采集", "Keep collecting in the background")} text={t(`请用「${title}」持续采集${platform}的推荐内容，后台运行。若已经在采集这个平台，不要重复启动；等我明确要求停止再停止。`, `Use ${title} to keep collecting recommended posts from ${platform} in the background. Do not start a duplicate if this platform is already running. Continue until I explicitly ask you to stop.`)} />
        <p className="text-sm leading-6 text-[var(--theme-text-muted)]">{t("没有关键词时明确说「推荐内容」；需要关键词时明确说「搜索」。是否显示浏览器取决于平台和登录状态；静默运行也不能跳过登录或验证码。以 Agent 返回的实际状态和限制为准。", "Say recommended posts when no keyword is needed, or explicitly request search. Browser visibility depends on the platform and login state; silent mode cannot bypass login or verification. Follow the actual status and limits reported by Agent.")}</p>
      </> : activity ? <>
        <label className="block text-sm">{t("要处理的媒体链接", "Media URL to process")}<input className={fieldClass} value={linkInput} onChange={event => setLinkInput(event.target.value)} maxLength={4096} placeholder="https://…" /></label>
        <Example t={t} label={t("下载媒体", "Download media")} text={t(`请用「${title}」下载这个链接里的媒体并把结果发给我：${link}`, `Use ${title} to download the media at this URL and send me the result: ${link}`)} />
        <Example t={t} label={t("提取文字", "Extract text")} text={t(`请用「${title}」处理这个链接，把图片上的文字和音频中的语音转成文字，整理排版，给我文本和 TXT 文件：${link}`, `Use ${title} to process this URL, extract text from its images and transcribe its audio, tidy the formatting, and return text plus a TXT file: ${link}`)} />
        <p className="text-sm leading-6 text-[var(--theme-text-muted)]">{t("要下载、转写或解读，请明确说出来；不要只说「处理一下」。这类任务处理你提供的链接，与另一类后台自动采集任务分开保存。", "Explicitly request download, transcription or interpretation instead of only saying process this. These tasks process the URLs you provide and are recorded separately from background discovery.")}</p>
      </> : <Example t={t} label={t("先检查，再执行", "Check before execution")} text={t(`我想使用「${title}」。请先检查它的能力和前提条件，告诉我还需要提供什么信息，再按我的要求执行并保存结果。`, `I want to use ${title}. Check its capabilities and prerequisites, tell me what information is needed, then carry out my request and save the results.`)} />}
    </section>
    <section className={sectionClass}>
      <h2 className="text-base font-semibold">{t("3. 查看进度和结果", "3. Check progress and results")}</h2>
      <Example t={t} label={t("查看当前状态", "Check current status")} text={collection ? t(`请查看「${title}」在${platform}的采集状态，告诉我已经保存多少条，是否有登录、验证或失败阻塞。`, `Check ${title} collection on ${platform}. Report saved records and any login, verification or failure blockers.`) : t(`请查看刚才「${title}」任务的实际状态、已完成内容和失败原因；不要重新启动。`, `Check the actual status, completed work and any failure of the last ${title} task. Do not restart it.`)} />
      <p className="text-sm leading-6 text-[var(--theme-text-body)]">{collection ? t("切回「查看结果」后，可按平台、类型、采集时间和标题文案查找。页面可见时会约每 10 秒检查更新，只有已经保存的内容才会出现，不必等整个采集结束。多图帖子合并为一条，点击图片可左右翻看，并下载单张或整组 ZIP。", "In Results, filter by platform, type, collection time or text. While visible, the page checks for updates about every 10 seconds. Saved items appear before the entire collection finishes. Multi-image posts share one card; open an image to browse and download individually or as a ZIP.") : activity ? t("切回「查看结果」后，可按通信端、状态、时间或请求内容查找。记录包括实际执行了此技能的任务及其可用结果和附件；只发消息但没有调用技能，不会生成这里的记录。", "In Results, filter by channel, status, time or request text. Records include tasks that actually executed this skill, with available results and attachments. A message that did not invoke the skill does not create an entry here.") : t("返回「查看结果」查看本应用已经保存的数据；以当前应用的数据和能力合同为准。", "Return to Results to review saved data under this application's data and capability contract.")}</p>
      {activity && app.task_channel_scope === "communication" ? <p className="text-sm leading-6">{t("当前应用只展示通信端任务，不包含 UI Agent 对话中的任务。", "This application currently shows channel tasks only, not UI Agent tasks.")}</p> : activity ? <p className="text-sm leading-6">{t("UI Agent 对话和已绑定通信端发起的任务都可展示；本页仍只显示当前技能实际处理过的任务。", "Tasks from UI Agent and linked channels can appear here, but only when this skill actually processed them.")}</p> : null}
    </section>
    <section className={sectionClass}>
      <h2 className="text-base font-semibold">{t("4. 停止任务", "4. Stop a task")}</h2>
      <Example t={t} label={t("停止并确认收尾", "Stop and confirm completion")} text={collection ? t(`请停止「${title}」在${platform}的持续采集，不要再启动下一轮；让正在处理的最后一条完成收尾，保留已经采集的内容，并告诉我是否已经停止。`, `Stop continuous ${title} collection on ${platform} and do not start another round. Let the current item finish gracefully, preserve saved data, and confirm whether it has stopped.`) : t(`请停止刚才「${title}」未完成的任务，保留已经保存的结果，并告诉我已停止、仍在收尾，还是无法取消。`, `Stop the unfinished ${title} task, preserve saved results, and tell me whether it stopped, is still finishing, or cannot be canceled.`)} />
      <p className="text-sm leading-6 text-[var(--theme-text-body)]">{t("有多个任务时，请带上平台、链接或 task_id，避免停错任务。收到「正在停止」不等于已经结束，可以再问一次状态。关闭网页、退出 AiAPP 或收起导航栏不会停止后台任务；停止与删除已保存数据也是两件事。", "With multiple tasks, include the platform, URL or task_id to identify the right one. Stopping is not the same as stopped; check status again if needed. Closing the page or AiAPP does not stop background work, and stopping is separate from deleting saved data.")}</p>
    </section>
    <section className={sectionClass}>
      <h2 className="text-base font-semibold">{t("5. 为什么还看不到结果？", "5. Why are results missing?")}</h2>
      <ul className="list-disc space-y-2 pl-5 text-sm leading-6 text-[var(--theme-text-body)]">
        <li>{t("先看 Agent 是否真正调用了对应技能，还是在询问信息、等待确认或报告依赖不可用。", "Check whether Agent actually called the skill or is asking a question, awaiting approval or reporting missing prerequisites.")}</li>
        <li>{t("任务可能还在登录、验证、下载或识别阶段。先查询状态，不要连续创建重复任务。", "The task may still be logging in, verifying, downloading or recognizing content. Check its status instead of creating duplicates.")}</li>
        <li>{t("清除筛选条件，再点击刷新内容；确认进入的是对应技能的 AiAPP。不同应用的数据不会混在一起。", "Clear filters and refresh Results. Make sure you opened the correct skill's AiAPP; different applications keep separate data.")}</li>
        <li>{t("图片文件已被清理、源链接失效或平台没有提供某项数据时，页面不能凭空补齐。可先重试预览，再让 Agent 检查或重新处理具体内容。", "Deleted image files, expired source links or unavailable platform fields cannot be reconstructed by the page. Retry the preview, then ask Agent to inspect or reprocess the specific item.")}</li>
      </ul>
      <p className="text-xs leading-5 text-[var(--theme-text-muted)]">{t("示例是自然语言请求，不是固定命令。是否可执行由当前技能能力、权限、平台状态和 Agent 实际判断决定；本页不会自行发送任务。", "Examples are natural-language requests, not fixed commands. Execution depends on current capabilities, permissions, platform state and Agent's decision. This page does not submit tasks automatically.")}</p>
    </section>
  </div>;
}
