# photo_organize Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the `photo_organize` implementation.

## Capability Summary
- `photo_organize` 按照片内的相机留存信息（优先 EXIF 的 `Make`、`Model`、`LensModel`、`FocalLength`、`DateTimeOriginal`）生成整理计划，或执行复制 / 移动整理。
- **读不到 EXIF 的照片不会做任何操作**，不会被落到 `unknown_*` 目录。
- **首次调用如果没有明确 `source_dir`，优先自动发现外接硬盘 / U 盘**：只发现 1 个外接盘根目录时直接使用该目录继续按 `plan` 预览；发现 0 个或多个时才发起询问并列出候选路径。
- 当前已显式支持 `macOS`、常规 `Linux` 与树莓派常见挂载点发现与路径提示；macOS 会发现 `/Volumes/<disk>` 并过滤系统根卷；Linux 会优先读取真实挂载点，并兼容 `/media/<user>/<disk>`、`/media/pi/<disk>`、`/mnt/<disk>`、`/mnt/usb0` 等路径；其他平台仍可手动传入绝对路径使用。
- 默认安全模式是 `plan`，只做预览，不直接改动文件。
- 整理层级默认是：`品牌 / 机型 / 镜头 / 焦段 / 年月`。
- 支持按需求动态改变目录层级：由 planner / LLM 将用户语义归一到结构化参数，如 `mode`、`group_by`、`capture_month`、`selected_brands`、`include_subdirs`、`preview_limit`。
- 技能内部不再维护自然语言语义词表；`args` 字符串或 object 里的 `text|prompt|input|instruction|query` 只作为兼容入口，用于提取显式路径或外接盘候选名。其他语义必须由 planner 传入结构化字段。
- 输出语言由 `configs/photo_organize.toml` 和 `configs/i18n/photo_organize.<locale>.toml` 控制，也可被 `args.locale/lang` 或 `context.locale/lang` 覆盖。

## Config Entry Points
- `configs/photo_organize.toml`: skill defaults such as locale and optional i18n file override.
- `configs/i18n/photo_organize.<locale>.toml`: user-visible message catalog for localized output.
- `configs/skills_registry.toml`: runtime registry entry, aliases, prompt file, risk/confirmation metadata, and planner visibility.
- No external account, API key, or model provider is required for the skill itself.

## Routing / Planner Contract
- `source_dir` is conditional, not a front-door blocker. If the user requests photo organization without a path, route to execution and call `organize` with `mode="plan"` unless the user explicitly asks only to list candidates.
- The skill owns the external-drive discovery step. It will auto-select a unique external drive / USB mount for preview, or return observed candidates when none or multiple are found.
- The normalizer/planner should ask the user for a path before execution only when the request explicitly conflicts with external-drive discovery, asks for a non-discoverable source, or requires an unsafe action that lacks required confirmation.

## Actions
- `prepare`
- `organize`
- Compatibility action aliases: `plan|preview|dry_run` behave as `organize` with default `mode="plan"`; `copy|move` behave as `organize` with matching default `mode`.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| `prepare` | none | no | - | - | 仅返回外接盘候选路径，并提示用户在候选不唯一时显式提供 `source_dir`。 |
| `organize` | `source_dir` | conditional | string(path) | - | 要整理的照片目录。若缺失，技能先尝试自动发现唯一外接盘根目录；发现 0 个或多个时转入询问流程；也可由自然语言中显式路径推断。 |
| `organize` | `mode` / `organize_mode` | no | string | `plan` | `plan|copy|move`。`plan` 只预览；`copy` 复制到整理目录；`move` 直接移动原文件。 |
| `plan` / `preview` / `dry_run` / `copy` / `move` | all `organize` params | conditional | object fields | action-derived mode | 兼容 planner 把预览或复制/移动意图放进 `action` 字段的结构化写法；仍建议优先使用 `action="organize"` + `mode`。 |
| `organize` | `output_dir` | no | string(path) | `<source_dir>/_organized_by_camera` | 整理后的输出目录。相对路径按 `source_dir` 解析。 |
| `organize` | `group_by` | no | string/string[] | `["brand","model","lens","focal_length","year_month"]` | 目录层级顺序。支持 `brand`、`model`、`lens`、`focal_length`、`year_month`。 |
| `organize` | `capture_month` | no | string | - | 仅整理指定月份拍摄的照片，格式建议 `YYYY-MM`。 |
| `organize` | `selected_brands` / `brands` | no | string/string[] | - | 仅整理指定品牌的照片；接受品牌名字符串或品牌名数组，其他品牌不动。 |
| `organize` | `include_subdirs` | no | bool | `true` | 是否递归扫描子目录。 |
| `organize` | `preview_limit` | no | integer | `12` | 返回的预览条目上限。 |
| all | `locale` / `lang` / `language` | no | string | config default | 输出语言，如 `zh-CN`、`en-US`。 |
| `organize` | `text` / `prompt` / `input` / `instruction` / `query` | no | string | - | 兼容自由文本入口；技能只从中提取显式路径或外接盘候选名，不推断模式、分组、日期、品牌或递归策略。 |
| all | raw string `args` | no | string | - | 兼容纯字符串入口；planner 应优先改写成结构化参数后调用。 |

## Success `extra` (`status=ok`)
- `prepare`:
  - `action = "prepare"`
  - `requires_user_input = true`
  - `missing_argument = "source_dir"`
  - `needs_directory = true`
  - `external_candidates`: 检测到的挂载点和常见照片子目录候选
- `organize` + `mode=plan`:
  - `source_dir`
  - `output_dir`
  - `photo_count`
  - `with_camera_metadata`
  - `without_camera_metadata`
  - `skipped_no_exif`
  - `group_by`
  - `capture_month`
  - `selected_brands`
  - `top_camera_groups`
  - `top_lens_groups`
  - `non_exif_files`
  - `preview`
- `organize` + `mode=copy|move`:
  - `processed`
  - `copied`
  - `moved`
  - `skipped`
  - `preview`

## Error Contract
- `args` 不是 object。
- `action` 不支持。
- `source_dir` 缺失且无法唯一发现外接盘时不报错，而是返回询问文本和外接盘候选路径。
- `source_dir` 不存在、不可访问或不是目录时返回可读 `error_text`。
- 指定目录下没有照片文件时返回可读 `error_text`。
- 若目录里有照片，但都读不到可识别 EXIF，会明确返回“本次不做操作”错误。
- 若指定了 `capture_month` 但该月份没有匹配照片，会返回明确的“该月份无照片”错误。
- 若指定了 `selected_brands` 但没有匹配品牌的照片，会返回明确的“该品牌无照片”错误。
- 自由文本里若未能唯一解析目录，且无法唯一发现外接盘，保持保守行为，继续要求用户明确指定目录；不要在技能内根据自然语言词表猜测整理模式或筛选条件。
- 执行 `copy|move` 时若发生部分失败，返回明确的失败统计和首条错误。

## Request/Response Examples
### Example 1 — 未发现或发现多个外接盘时询问
Request:
```json
{"request_id":"demo-1","args":{"action":"organize"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"未能唯一确定要整理的照片目录。\n\n当前检测到的外接硬盘 / U 盘候选路径：\n1. /media/pi/SDCARD\n2. /mnt/photo-disk\n\n请重新调用 `photo_organize`，显式传入 `source_dir`。建议先用 `mode=\"plan\"` 预览，再决定是否 `copy` 或 `move`。","buttons":[{"text":"/media/pi/SDCARD","value":"{\"action\":\"organize\",\"source_dir\":\"/media/pi/SDCARD\",\"mode\":\"plan\"}"}],"extra":{"action":"prepare","requires_user_input":true,"missing_argument":"source_dir","needs_directory":true,"external_candidates":["/media/pi/SDCARD","/mnt/photo-disk"],"recommended_mode":"plan"},"error_text":null}
```

### Example 2 — 唯一外接盘自动进入预览
Request:
```json
{"request_id":"demo-2","args":{"action":"organize","mode":"plan","preview_limit":3}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"已完成照片整理预览：共扫描 128 张照片，其中 120 张读到了可识别的 EXIF 信息，跳过 8 张无可识别 EXIF 的照片。整理目标目录：/media/pi/SDCARD/_organized_by_camera。","extra":{"action":"organize","mode":"plan","source_dir":"/media/pi/SDCARD","output_dir":"/media/pi/SDCARD/_organized_by_camera","photo_count":128,"with_camera_metadata":120,"without_camera_metadata":8,"skipped_no_exif":8,"top_camera_groups":[{"camera":"Canon / EOS R6","count":64}],"top_lens_groups":[{"lens":"RF24-70mm F2.8 L IS USM / 35mm","count":42}],"preview":[{"source":"IMG_0001.JPG","destination":"Canon/EOS R6/RF24-70mm F2.8 L IS USM/35mm/2026-03/IMG_0001.JPG","classification_path":"Canon/EOS R6/RF24-70mm F2.8 L IS USM/35mm/2026-03"}]},"error_text":null}
```

### Example 3 — 显式路径预览整理计划
Request:
```json
{"request_id":"demo-3","args":{"action":"organize","source_dir":"/Volumes/SDCARD/DCIM","mode":"plan","preview_limit":3}}
```
Response:
```json
{"request_id":"demo-3","status":"ok","text":"Preview generated for photo organization: scanned 128 photos, 120 with readable EXIF metadata, skipped 8 without readable EXIF. Output directory: /Volumes/SDCARD/DCIM/_organized_by_camera.","extra":{"action":"organize","mode":"plan","source_dir":"/Volumes/SDCARD/DCIM","output_dir":"/Volumes/SDCARD/DCIM/_organized_by_camera","photo_count":128,"with_camera_metadata":120,"without_camera_metadata":8,"skipped_no_exif":8,"top_camera_groups":[{"camera":"Canon / EOS R6","count":64}],"top_lens_groups":[{"lens":"RF24-70mm F2.8 L IS USM / 35mm","count":42}],"preview":[{"source":"IMG_0001.JPG","destination":"Canon/EOS R6/RF24-70mm F2.8 L IS USM/35mm/2026-03/IMG_0001.JPG","classification_path":"Canon/EOS R6/RF24-70mm F2.8 L IS USM/35mm/2026-03"}]},"error_text":null}
```

### Example 4 — 执行复制整理
Request:
```json
{"request_id":"demo-4","args":{"action":"organize","source_dir":"/Volumes/SDCARD/DCIM","mode":"copy"}}
```
Response:
```json
{"request_id":"demo-4","status":"ok","text":"已完成照片整理：共扫描 128 张，跳过 8 张无可识别 EXIF 的照片，实际处理 120 张，按品牌/机型/镜头/焦段/年月复制 120 张，跳过 0 张。输出目录：/Volumes/SDCARD/DCIM/_organized_by_camera。","extra":{"action":"organize","mode":"copy","processed":120,"copied":120,"moved":0,"skipped":0,"skipped_no_exif":8},"error_text":null}
```

### Example 5 — planner 归一后的结构化请求
Request:
```json
{"request_id":"demo-5","args":{"action":"organize","source_dir":"/Volumes/SDCARD/DCIM","mode":"plan","preview_limit":5}}
```
Response:
```json
{"request_id":"demo-5","status":"ok","text":"已完成照片整理预览：共扫描 128 张照片，其中 120 张读到了可识别的 EXIF 信息，跳过 8 张无可识别 EXIF 的照片。整理目标目录：/Volumes/SDCARD/DCIM/_organized_by_camera。","extra":{"action":"organize","mode":"plan","source_dir":"/Volumes/SDCARD/DCIM","preview":[{"source":"IMG_0001.JPG","destination":"Canon/EOS R6/RF24-70mm F2.8 L IS USM/35mm/2026-03/IMG_0001.JPG"}]},"error_text":null}
```

### Example 6 — 产品式表达经 planner 归一
Request:
```json
{"request_id":"demo-6","args":{"action":"organize","mode":"plan","selected_brands":["Canon","Sony"],"group_by":["lens","year_month"],"capture_month":"2026-04"}}
```
Response:
```json
{"request_id":"demo-6","status":"ok","text":"已完成照片整理预览：...","extra":{"action":"organize","mode":"plan","group_by":["lens","year_month"],"capture_month":"2026-04"},"error_text":null}
```

### Example 7 — 品牌筛选 + 无 EXIF 清单
Request:
```json
{"request_id":"demo-7","args":{"action":"organize","source_dir":"/Volumes/SDCARD/DCIM","selected_brands":["Canon","Sony"],"mode":"plan"}}
```
Response:
```json
{"request_id":"demo-7","status":"ok","text":"已完成照片整理预览：...","extra":{"action":"organize","selected_brands":["Canon","Sony"],"non_exif_files":["MISC/IMG_9999.JPG"]},"error_text":null}
```
