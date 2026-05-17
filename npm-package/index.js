const path = require("node:path");
const os = require("node:os");

function binaryPath() {
  const suffix = os.type() === "Windows_NT" ? ".exe" : "";
  return path.join(__dirname, "vendor", `gh-actions-updater${suffix}`);
}

module.exports = { binaryPath };
