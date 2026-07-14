import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import test from "node:test";
import fs from "node:fs";
import crypto from "node:crypto";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const cli = path.join(repoRoot, "bin", "receipts.mjs");

function command(args) {
  return spawnSync(process.execPath, [cli, ...args], {
    cwd: repoRoot,
    encoding: "utf8",
  });
}

function directoryDigest(root) {
  const hash = crypto.createHash("sha256");
  const visit = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      const full = path.join(dir, entry.name);
      const relative = path.relative(root, full).replaceAll(path.sep, "/");
      hash.update(`${entry.isDirectory() ? "d" : "f"}:${relative}\0`);
      if (entry.isDirectory()) visit(full);
      else hash.update(fs.readFileSync(full));
    }
  };
  visit(root);
  return hash.digest("hex");
}

test("an unbound critical claim suppresses a provisional false-green probability", (t) => {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "receipts-score-suppressed-"));
  t.after(() => {
    if (process.env.RECEIPTS_KEEP_CARD_FIXTURE === "1") {
      process.stdout.write(`visual fixture: ${fixture}\n`);
    } else {
      fs.rmSync(fixture, { recursive: true, force: true });
    }
  });
  const project = path.join(fixture, "project");
  const runDir = path.join(fixture, "run");
  const runs = path.join(fixture, "calibration-runs");
  const imports = path.join(fixture, "imports");
  const source = path.join(fixture, "tasks.json");
  const bundle = path.join(fixture, "calibration.bundle.json");
  fs.mkdirSync(project, { recursive: true });
  fs.mkdirSync(runs);
  fs.writeFileSync(path.join(project, "subject.txt"), "claimed but unchecked\n");
  assert.equal(command(["init", runDir, "--repo-root", project]).status, 0);
  fs.mkdirSync(path.join(runDir, "session"), { recursive: true });
  fs.writeFileSync(path.join(runDir, "session", "generic.json"), `${JSON.stringify({
    provider: "openai",
    requested_model: "gpt-5",
    resolved_model_snapshot: "gpt-5-2026-07-01",
    agent_name: "codex",
    agent_version: "1.2.3",
  })}\n`);
  assert.equal(command(["session", "capture", "--run-dir", runDir, "--adapter", "generic"]).status, 0);
  const lane = path.join(fixture, "lane.md");
  fs.writeFileSync(lane, '```receipts-evidence-jsonl\n{"id":"ev-critical","kind":"observation","summary":"subject is complete","source_ids":["file:subject.txt:1"]}\n```\n');
  assert.equal(command(["ingest", "--run-dir", runDir, "--lane", "worker", "--agent-id", "codex", "--from", lane]).status, 0);
  assert.equal(command(["compile", "--run-dir", runDir]).status, 0);

  const records = Array.from({ length: 120 }, (_, index) => ({
    task_id: `task-${index}`,
    result: index % 5 === 0 ? "failure" : "success",
    provider: "openai",
    model_snapshot: "gpt-5-2026-07-01",
    agent_name: "codex",
    agent_version: "1.2.3",
    task_family: "observation",
    repository_id: `repo-${index % 10}`,
    language: "unknown",
  }));
  fs.writeFileSync(source, `${JSON.stringify({
    format_version: "1",
    data_kind: "task-results",
    source_url: "https://example.invalid/score-fixture/v1",
    retrieval_date: "2026-07-14",
    methodology_version: "method-v1",
    harness_version: "harness-v1",
    sample_size: records.length,
    attribution: "Score fixture authors",
    license: "CC-BY-4.0",
    records,
  })}\n`);
  assert.equal(command(["import-eval", "--from", source, "--out", imports]).status, 0);
  assert.equal(command(["calibration", "build", "--runs", runs, "--imports", imports, "--out", bundle]).status, 0);

  const score = command(["score", "--run-dir", runDir, "--bundle", bundle]);
  assert.equal(score.status, 0, `${score.stdout}\n${score.stderr}`);
  const report = JSON.parse(score.stdout);
  assert.equal(report.score_status, "suppressed");
  assert.equal(report.false_green_probability, null);
  assert.equal(report.upper_95_false_green_risk, null);
  assert.ok(report.suppression_reasons.some((reason) => reason.includes("ev-critical") && reason.includes("unbound")));
  assert.equal(report.calibration_status, "provisional");

  const manifest = JSON.parse(fs.readFileSync(path.join(runDir, "manifest.json"), "utf8"));
  const missingConsent = command([
    "publish", "--run-dir", runDir, "--consent", path.join(fixture, "missing-consent.json"), "--out", path.join(fixture, "missing-out"),
  ]);
  assert.notEqual(missingConsent.status, 0, "publication without a consent file must fail closed");

  const consent = path.join(fixture, "consent.json");
  fs.writeFileSync(consent, `${JSON.stringify({
    format_version: "1",
    consent: true,
    run_id: manifest.run_id,
    calibration_bundle: bundle,
    public_data_version: "v1",
    license: "CC-BY-4.0",
    authorized_by: "fixture-reviewer",
  })}\n`);
  const publishedA = path.join(fixture, "published-a");
  const publishedB = path.join(fixture, "published-b");
  const firstPublish = command(["publish", "--run-dir", runDir, "--consent", consent, "--out", publishedA]);
  assert.equal(firstPublish.status, 0, `${firstPublish.stdout}\n${firstPublish.stderr}`);
  const secondPublish = command(["publish", "--run-dir", runDir, "--consent", consent, "--out", publishedB]);
  assert.equal(secondPublish.status, 0, `${secondPublish.stdout}\n${secondPublish.stderr}`);
  assert.equal(directoryDigest(publishedA), directoryDigest(publishedB), "public projection must be byte-stable");

  const publishedText = fs.readFileSync(path.join(publishedA, "v1", `${manifest.run_id}.json`), "utf8");
  assert.doesNotMatch(publishedText, /claimed but unchecked|subject\.txt|C:\\|repo_root|stdout|stderr|prompt/i);
  assert.match(publishedText, /CC-BY-4\.0/);

  const secretConsent = path.join(fixture, "secret-consent.json");
  fs.writeFileSync(secretConsent, `${JSON.stringify({
    format_version: "1",
    consent: true,
    run_id: manifest.run_id,
    calibration_bundle: bundle,
    public_data_version: "v1",
    license: "CC-BY-4.0",
    authorized_by: "github_pat_11AA_fake_secret_fixture",
  })}\n`);
  const secretPublish = command(["publish", "--run-dir", runDir, "--consent", secretConsent, "--out", path.join(fixture, "secret-out")]);
  assert.notEqual(secretPublish.status, 0);
  assert.match(secretPublish.stderr, /secret/i);

  const cardsA = path.join(fixture, "cards-a");
  const cardsB = path.join(fixture, "cards-b");
  const firstCards = command(["cards", "build", "--data", publishedA, "--out", cardsA]);
  assert.equal(firstCards.status, 0, `${firstCards.stdout}\n${firstCards.stderr}`);
  const secondCards = command(["cards", "build", "--data", publishedA, "--out", cardsB]);
  assert.equal(secondCards.status, 0, `${secondCards.stdout}\n${secondCards.stderr}`);
  assert.equal(directoryDigest(cardsA), directoryDigest(cardsB), "static JSON and HTML cards must be byte-stable");
  assert.match(fs.readFileSync(path.join(cardsA, "index.html"), "utf8"), /Agent Reliability Cards/);
  assert.match(fs.readFileSync(path.join(cardsA, "cards.json"), "utf8"), /upper_95_false_green_risk/);

  const taskMix = path.join(fixture, "task-mix.json");
  fs.writeFileSync(taskMix, `${JSON.stringify({
    format_version: "1",
    release_id: "reliability-index-v1",
    families: [
      { id: "observation", version: "v1" },
      { id: "bugfix", version: "v1" },
      { id: "refactor", version: "v1" },
    ],
  })}\n`);
  const indexOut = path.join(fixture, "index-release");
  const indexBuild = command(["index", "build", "--data", publishedA, "--task-mix", taskMix, "--out", indexOut]);
  assert.equal(indexBuild.status, 0, `${indexBuild.stdout}\n${indexBuild.stderr}`);
  const indexReport = JSON.parse(fs.readFileSync(path.join(indexOut, "reliability-index.json"), "utf8"));
  assert.equal(indexReport.release_status, "withheld");
  assert.equal(indexReport.variants[0].eligible, false);
  assert.equal(indexReport.variants[0].reliability_index, null);
  assert.ok(indexReport.variants[0].ineligibility_reasons.some((reason) => reason.includes("calibrated")));
});
