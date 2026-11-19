import { useEffect, useRef, useState } from "react";
import { readJsonApiResponse } from "../lib/api-response";
import { formatBytes } from "../lib/display-format";
import { useRequestOwner } from "../hooks/useRequestOwner";
import type { ApiResponse } from "../types/api";
type Translate = (zh: string, en: string) => string;
interface Category { id: string; bytes: number; files: number; partial: boolean; protected: boolean }
interface Inventory { categories: Category[]; collected_at_ts: number; history: Array<{ collected_at_ts: number; categories: Category[] }>; result_compaction: { compacted_rows: number; saved_logical_bytes: number; database_reusable_bytes?: number } }
const labels: Record<string, [string, string]> = {user_outputs:["任务文件","Task files"],validation_evidence:["验证资料","Validation evidence"],operator_verification:["操作验证资料","Operator evidence"],packages:["已安装技能包","Installed skill packages"],skill_data:["技能数据","Skill data"],logs:["日志","Logs"],models:["本地模型","Local models"],backups:["备份","Backups"],database:["任务与运行数据库","Task and runtime database"],release_backups:["升级回滚备份","Upgrade rollback backups"],release_downloads:["升级下载包","Downloaded upgrade packages"],prebuilt_packages:["发行技能包","Release skill packages"],runtime_dependencies:["运行依赖","Runtime dependencies"],runtime_binaries:["运行程序","Runtime programs"]};
export function StorageGovernancePanel({t,apiFetch,requestScope}: { t:Translate; apiFetch:(path:string,init?:RequestInit)=>Promise<Response>;requestScope:string }) {
 const current=useRequestOwner(requestScope);
 const [inventory,setInventory]=useState<Inventory|null>(null);
 const [busy,setBusy]=useState(false);const [error,setError]=useState(false);const [message,setMessage]=useState<string|null>(null);
 const flight=useRef(false);
 useEffect(()=>{setInventory(null);setError(false);setBusy(false);setMessage(null);flight.current=false;},[requestScope]);
 async function refresh(compact=false) {
  if (!current() || flight.current) return;flight.current=true;setBusy(true);setError(false);setMessage(null);
  try {
   if(compact) {
    const res=await apiFetch("/v1/system/storage-compact",{method:"POST"});
    const response=await readJsonApiResponse<ApiResponse<{scanned_rows:number;compacted_rows:number;saved_logical_bytes:number}>>(res);
    if(!current()) return;
    if(!res.ok||!response.ok||!response.data) throw new Error("result_compaction_failed");
    setMessage(response.data.scanned_rows===0?t("历史结果已检查完毕。","Historical results have been checked."):t(`本批已检查 ${response.data.scanned_rows} 条，压缩 ${response.data.compacted_rows} 条。可继续处理下一批。`,`Checked ${response.data.scanned_rows} records and compacted ${response.data.compacted_rows}. You can process the next batch.`));
   }
   if(!current()) return;
   const res=await apiFetch("/v1/system/storage-scan",{method:"POST"});
   const response=await readJsonApiResponse<ApiResponse<Inventory>>(res);
   if(!current()) return;
   if(!res.ok||!response.ok||!response.data) throw new Error("storage_scan_failed");
   setInventory(response.data);
  } catch {if(current())setError(true);} finally {if(current()){flight.current=false;setBusy(false);}}
 }
 return <section className="theme-panel-soft p-4 sm:p-5">
  <div className="flex flex-wrap items-center justify-between gap-3"><div><h3 className="font-semibold">{t("存储管理","Storage management")}</h3><p className="mt-1 text-xs opacity-60">{t("查看主要目录的文件大小，并无损压缩历史结果。任务文件和验证资料会保留。","Review file sizes in major directories and compress historical results without loss. Task files and evidence are retained.")}</p></div><button className="theme-secondary-btn px-3 py-2 text-sm" disabled={busy||!requestScope} onClick={()=>void refresh()}>{busy?t("处理中","Working"):t("查看或刷新占用","Review or refresh usage")}</button></div>
  {error&&<p role="alert" className="mt-3 text-sm">{t("暂时无法读取或压缩，请检查服务连接后重试。","Could not read or compact storage. Check the connection and retry.")}</p>}
  {inventory&&<><div className="mt-4 divide-y divide-white/8">{inventory.categories.map(category=>{
   const prior=inventory.history.slice(1).find(sample=>sample.collected_at_ts<inventory.collected_at_ts)?.categories.find(c=>c.id===category.id);
   const delta=prior&&!prior.partial&&!category.partial?category.bytes-prior.bytes:null;
   return <div className="flex items-center justify-between gap-3 py-2 text-sm" key={category.id}><span>{labels[category.id]?t(...labels[category.id]):category.id}<span className="ml-2 text-xs opacity-50">{t("保留","Retained")}</span></span><span>{category.partial?t("至少 ","At least "):""}{formatBytes(category.bytes)}{delta!==null&&delta!==0&&<span className="ml-2 text-xs opacity-60">{delta>0?"+":"−"}{formatBytes(Math.abs(delta))}</span>}</span></div>;
  })}</div><p className="mt-3 text-xs opacity-60">{t(`已压缩 ${inventory.result_compaction.compacted_rows} 条历史结果，减少逻辑存储 ${formatBytes(inventory.result_compaction.saved_logical_bytes)}。数据库可复用空间，文件大小不一定立即下降。`,`Compacted ${inventory.result_compaction.compacted_rows} historical results, saving ${formatBytes(inventory.result_compaction.saved_logical_bytes)} of logical storage. The database can reuse space; its file may not shrink immediately.`)}</p>{inventory.result_compaction.database_reusable_bytes !== undefined && <p className="mt-2 text-xs opacity-60">{t(`数据库已有 ${formatBytes(inventory.result_compaction.database_reusable_bytes)} 可复用页空间。`, `The database has ${formatBytes(inventory.result_compaction.database_reusable_bytes)} of reusable free pages.`)}</p>}<p className="mt-2 text-xs opacity-60">{t("增减量与上次完整扫描比较。标有“至少”的项目为部分统计；这里没有可自动删除的任务文件。","Changes compare with the previous complete scan. “At least” indicates a partial count. Task files are excluded from automatic deletion.")}</p><button className="theme-secondary-btn mt-3 px-3 py-2 text-sm" disabled={busy} onClick={()=>void refresh(true)}>{t("压缩下一批历史结果（最多 64 条）","Compact next batch of historical results (up to 64)")}</button></>}
  {message&&<p role="status" className="mt-3 text-sm opacity-75">{message}</p>}
 </section>;
}
