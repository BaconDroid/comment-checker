#!/usr/bin/env node

const { spawnSync } = require("node:child_process");
const { existsSync } = require("node:fs");
const { dirname, join, resolve } = require("node:path");

function scriptPath() {
  return resolve(dirname(__filename), "..", "..", "scripts", "doctor.sh");
}

function runDoctor(options = {}) {
  const script = options.script || scriptPath();
  const run = options.run || spawnSync;
  const stderr = options.stderr || process.stderr;
  if (!existsSync(script)) {
    stderr.write(`[comment-checker] doctor: scripts/doctor.sh not found at ${script}\n`);
    stderr.write("[comment-checker] doctor: this wrapper only works from a source checkout.\n");
    return 1;
  }
  const result = run("bash", [script], {
    cwd: options.cwd || process.cwd(),
    stdio: options.stdio || "inherit",
    env: options.env || process.env,
  });
  if (result.error) {
    stderr.write(`[comment-checker] doctor: failed to start bash: ${result.error.message}\n`);
    return 1;
  }
  return result.status === null ? 1 : result.status;
}

if (require.main === module) {
  process.exit(runDoctor());
}

module.exports = { runDoctor, scriptPath };
