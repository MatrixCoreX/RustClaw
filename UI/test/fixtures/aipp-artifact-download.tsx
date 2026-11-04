import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { TaskArtifactCard } from "../../src/components/ChatPage";
import { AippTaskActivityCard } from "../../src/components/AippPage";
import type { AippTaskActivityItem, TaskArtifact } from "../../src/types/api";
import "../../src/index.css";

const params = new URLSearchParams(location.search);
document.documentElement.dataset.theme = params.get("theme") || "dark";
const artifact: TaskArtifact = {schema_version:2,id:"file",artifact_ref:"artifact:task/download-test/file",filename:"report.txt",kind:"file",mime_type:"text/plain",size_bytes:16,sha256:"a".repeat(64),download_url:"/v1/tasks/download-test/artifacts/file/content",preview_url:null};
const item: AippTaskActivityItem = {schema_version:1,sequence:1,task_id:"download-test",channel:"ui",status:"succeeded",actions:[],input_text:"Download a file",result_text:"Saved",error_text:null,source_urls:[],created_at:"1788912000",updated_at:"1788912000",event_at_ms:1788912000000,artifacts:[{...artifact,preview_url:null}]};
const t = (_zh: string, en: string) => en;
const apiFetch = (path: string, init?: RequestInit) => fetch(path, init);
function Fixture() {
  const [visible, setVisible] = useState(true);
  return <main className="theme-shell min-h-screen p-4"><div className="mx-auto max-w-3xl">
    <button type="button" className="theme-secondary-btn mb-4 px-3 py-2" onClick={() => setVisible(value => !value)}>{visible ? "Remove card" : "Show card"}</button>
    {visible ? params.get("view") === "chat" ? <TaskArtifactCard artifact={artifact} artifactFetch={apiFetch} t={t} /> : <AippTaskActivityCard item={item} apiFetch={apiFetch} t={t} lang="en" /> : null}
  </div></main>;
}
createRoot(document.getElementById("root")!).render(<StrictMode><Fixture /></StrictMode>);
