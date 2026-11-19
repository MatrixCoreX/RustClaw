import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
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
