const assert = require("node:assert/strict");
const { test } = require("node:test");

const { runDoctor, scriptPath } = require("./doctor");

function fakeRun(result) {
  const calls = [];
  const run = (command, args, options) => {
    calls.push({ command, args, options });
    return result;
  };
  run.calls = calls;
  return run;
}

function fakeStderr() {
  const lines = [];
  return { lines, write: (line) => lines.push(line) };
}

test("runDoctor forwards the exit status of the shell script", () => {
  const run = fakeRun({ status: 2, stdout: "", stderr: "" });
  const status = runDoctor({ run, stderr: fakeStderr(), cwd: "/somewhere" });
  assert.equal(status, 2);
  assert.equal(run.calls.length, 1);
  assert.deepEqual(run.calls[0].args, [scriptPath()]);
  assert.equal(run.calls[0].options.cwd, "/somewhere");
});

test("runDoctor reports a missing scripts/doctor.sh and exits non-zero", () => {
  const stderr = fakeStderr();
  const status = runDoctor({
    run: fakeRun({ status: 0 }),
    stderr,
    script: "/nonexistent/doctor.sh",
  });
  assert.equal(status, 1);
  assert.ok(stderr.lines.some((line) => line.includes("scripts/doctor.sh not found")));
});

test("runDoctor treats a spawn failure as a failure instead of success", () => {
  const stderr = fakeStderr();
  const status = runDoctor({
    run: fakeRun({ error: new Error("bash missing") }),
    stderr,
  });
  assert.equal(status, 1);
  assert.ok(stderr.lines.some((line) => line.includes("failed to start bash")));
});
