import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { StorageGovernancePanel } from "../components/StorageGovernancePanel";
import { renderToStaticMarkup } from "react-dom/server";
import { SkillPolicyReviewPanel } from "../components/SkillPolicyReviewPanel";
import type { SkillPolicyReview } from "../types/api";
const review: SkillPolicyReview = {schema_version:1,skill_name:"permission_probe",package_version:"1.0.0",manifest_digest:"a".repeat(64),capability_request_digest:"b".repeat(64),registry_generation:4,admission_state:"awaiting_policy_approval",risk_level:"high",permissions:{network:true,filesystem_read:false,filesystem_write:true,subprocess:false,package_install:false,privilege_escalation:false,external_publish:false,llm_gateway:false,credential_refs:["private-provider-credential-ref"]},capabilities:[{name:"probe",action:"run",effect:"mutate"}]};
test("permission review distinguishes installed and approved and discloses required access without credential references",()=>{
 const markup=renderToStaticMarkup(<SkillPolicyReviewPanel t={(zh)=>zh} reviews={[review]} loading={false} onDecision={()=>undefined}/>);
 assert.match(markup,/已安装，等待授权/);assert.match(markup,/修改文件/);assert.match(markup,/访问网络/);assert.match(markup,/执行时仍需要确认/);assert.match(markup,/批准权限并开启/);assert.doesNotMatch(markup,/private-provider-credential-ref/);assert.doesNotMatch(markup,/撤销授权/);
});
test("approved skill offers revocation and is translated into English",()=>{
 const markup=renderToStaticMarkup(<SkillPolicyReviewPanel t={(_,en)=>en} reviews={[{...review,admission_state:"enabled"}]} loading={false} onDecision={()=>undefined}/>);
 assert.match(markup,/Approved and enabled/);assert.match(markup,/Revoke approval/);assert.doesNotMatch(markup,/Approve permissions and enable/);
});

const inventory = { collected_at_ts:20, categories:[{id:"user_outputs",bytes:4096,files:1,partial:true,protected:true}],history:[],result_compaction:{compacted_rows:2,saved_logical_bytes:32000} };
const english=(_zh:string,en:string)=>en;
test("storage review is manual, reports partial counts and serializes compaction",async()=>{
 globalThis.IS_REACT_ACT_ENVIRONMENT=true;
 const calls:string[]=[];
 const apiFetch=async(path:string)=>{calls.push(path);return new Response(JSON.stringify({ok:true,data:path.endsWith("storage-compact")?{scanned_rows:64,compacted_rows:2,saved_logical_bytes:32000}:inventory}));};
 let renderer!:ReactTestRenderer;
 await act(async()=>{renderer=create(<StorageGovernancePanel t={english} apiFetch={apiFetch} requestScope="owner-a"/>);});
 try {
  assert.deepEqual(calls,[]);
  await act(async()=>{renderer.root.findByType("button").props.onClick();});
  assert.deepEqual(calls,["/v1/system/storage-scan"]);
  assert.match(JSON.stringify(renderer.toJSON()),/At least/);assert.match(JSON.stringify(renderer.toJSON()),/Retained/);
  await act(async()=>{const button=renderer.root.findAllByType("button")[1];button.props.onClick();button.props.onClick();});
  assert.deepEqual(calls,["/v1/system/storage-scan","/v1/system/storage-compact","/v1/system/storage-scan"]);
 } finally {await act(async()=>{renderer.unmount();});}
});
test("changing server identity prevents a stale compaction from scanning the next server",async()=>{
 globalThis.IS_REACT_ACT_ENVIRONMENT=true;
 let resolve!:(response:Response)=>void;const pending=new Promise<Response>(done=>{resolve=done;});const calls:string[]=[];
 const apiFetch=async(path:string)=>{calls.push(path);return path.endsWith("storage-compact")?pending:new Response(JSON.stringify({ok:true,data:inventory}));};
 let renderer!:ReactTestRenderer;
 await act(async()=>{renderer=create(<StorageGovernancePanel t={english} apiFetch={apiFetch} requestScope="owner-a"/>);});
 try {
  await act(async()=>{renderer.root.findByType("button").props.onClick();});
  await act(async()=>{renderer.root.findAllByType("button")[1].props.onClick();});
  await act(async()=>{renderer.update(<StorageGovernancePanel t={english} apiFetch={apiFetch} requestScope="owner-b"/>);});
  await act(async()=>{resolve(new Response(JSON.stringify({ok:true,data:{scanned_rows:1,compacted_rows:1,saved_logical_bytes:2048}})));});
  assert.deepEqual(calls,["/v1/system/storage-scan","/v1/system/storage-compact"]);
  assert.doesNotMatch(JSON.stringify(renderer.toJSON()),/Compacted 2 historical/);
 } finally {await act(async()=>{renderer.unmount();});}
});
