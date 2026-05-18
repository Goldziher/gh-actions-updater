const fs = require("node:fs");
const crypto = require("node:crypto");
const http = require("node:http");
const https = require("node:https");
const os = require("node:os");
const path = require("node:path");
const AdmZip = require("adm-zip");
const tar = require("tar");

const { version } = require("./package.json");

const ASSET_NAME = "gh-actions-updater";
const BINARY_NAME = "gau";
const REPO = "Goldziher/gh-actions-updater";

function platformTriple() {
  const type = os.type();
  const arch = os.arch();

  if (type === "Windows_NT") {
    if (arch === "x64") return "x86_64-pc-windows-gnu";
    throw new Error("32-bit Windows is not supported");
  }

  if (type === "Linux") {
    if (arch === "x64") return "x86_64-unknown-linux-gnu";
    if (arch === "arm64") return "aarch64-unknown-linux-gnu";
  }

  if (type === "Darwin") {
    if (arch === "x64") return "x86_64-apple-darwin";
    if (arch === "arm64") return "aarch64-apple-darwin";
  }

  throw new Error(`Unsupported platform: ${type} ${arch}`);
}

function releaseTag() {
  return `v${version}`;
}

function binaryUrl() {
  const triple = platformTriple();
  const extension = triple.includes("windows") ? "zip" : "tar.gz";
  return `https://github.com/${REPO}/releases/download/${releaseTag()}/${ASSET_NAME}-${triple}.${extension}`;
}

function checksumsUrl() {
  return `https://github.com/${REPO}/releases/download/${releaseTag()}/checksums.txt`;
}

function binaryFilename() {
  return os.type() === "Windows_NT" ? `${BINARY_NAME}.exe` : BINARY_NAME;
}

function download(url, destination, redirects = 5) {
  return new Promise((resolve, reject) => {
    if (redirects === 0) {
      reject(new Error("Too many redirects"));
      return;
    }

    const client = url.startsWith("https:") ? https : http;
    const request = client.get(
      url,
      { headers: { "User-Agent": "gh-actions-updater-npm-wrapper" } },
      (response) => {
        if (
          response.statusCode >= 300 &&
          response.statusCode < 400 &&
          response.headers.location
        ) {
          download(response.headers.location, destination, redirects - 1)
            .then(resolve)
            .catch(reject);
          return;
        }

        if (response.statusCode !== 200) {
          reject(new Error(`HTTP ${response.statusCode}: ${response.statusMessage}`));
          return;
        }

        const file = fs.createWriteStream(destination);
        response.pipe(file);
        file.on("finish", () => file.close(resolve));
        file.on("error", (error) => {
          fs.unlink(destination, () => {});
          reject(error);
        });
      },
    );

    request.on("error", reject);
    request.setTimeout(30000, () => {
      request.destroy();
      reject(new Error("Download timeout"));
    });
  });
}

async function extract(archivePath, destinationDir) {
  const binary = binaryFilename();
  if (archivePath.endsWith(".zip")) {
    const zip = new AdmZip(archivePath);
    const entry = zip.getEntries().find((item) => item.entryName.endsWith(binary));
    if (!entry) throw new Error("Binary not found in downloaded archive");
    zip.extractEntryTo(entry, destinationDir, false, true);
    return;
  }

  await tar.extract({
    file: archivePath,
    cwd: destinationDir,
    filter: (entryPath) => entryPath.endsWith(binary),
  });
}

function sha256(filePath) {
  const hash = crypto.createHash("sha256");
  hash.update(fs.readFileSync(filePath));
  return hash.digest("hex");
}

function expectedChecksum(checksumsPath, archivePath) {
  const archiveName = path.basename(archivePath);
  const checksums = fs.readFileSync(checksumsPath, "utf8").split(/\r?\n/);
  for (const line of checksums) {
    const [checksum, filename] = line.trim().split(/\s+/);
    if (filename === archiveName) return checksum;
  }
  throw new Error(`Checksum not found for ${archiveName}`);
}

function verifyChecksum(archivePath, checksumsPath) {
  const expected = expectedChecksum(checksumsPath, archivePath);
  const actual = sha256(archivePath);
  if (actual !== expected) {
    throw new Error(`Checksum mismatch for ${path.basename(archivePath)}`);
  }
}

async function install() {
  const vendorDir = path.join(__dirname, "vendor");
  const binaryPath = path.join(vendorDir, binaryFilename());
  const url = binaryUrl();
  const archivePath = path.join(vendorDir, url.endsWith(".zip") ? "download.zip" : "download.tar.gz");
  const releaseArchivePath = path.join(vendorDir, path.basename(url));
  const checksumsPath = path.join(vendorDir, "checksums.txt");

  fs.mkdirSync(vendorDir, { recursive: true });
  if (fs.existsSync(binaryPath)) return;

  console.log(`Downloading ${BINARY_NAME} from ${url}...`);
  await download(url, releaseArchivePath);
  await download(checksumsUrl(), checksumsPath);
  verifyChecksum(releaseArchivePath, checksumsPath);
  fs.renameSync(releaseArchivePath, archivePath);
  await extract(archivePath, vendorDir);
  fs.unlinkSync(archivePath);
  fs.unlinkSync(checksumsPath);

  if (os.type() !== "Windows_NT") {
    fs.chmodSync(binaryPath, 0o755);
  }
}

install().catch((error) => {
  console.error(`Error installing ${BINARY_NAME}: ${error.message}`);
  process.exit(1);
});
