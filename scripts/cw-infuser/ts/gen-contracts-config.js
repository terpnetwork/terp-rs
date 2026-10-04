const fs = require('fs');
const path = require('path');

function toPascalCase(str) {
  return str
    .split(/[-_]/)
    .map(word => word.charAt(0).toUpperCase() + word.slice(1).toLowerCase())
    .join('');
}

const rootDir = path.resolve(__dirname, '../..');
const contractsDir = path.join(rootDir, 'contracts');
const vendorDir = path.join(rootDir, 'vendor');
const outputDir = __dirname;
const outputFile = path.join(outputDir, 'contracts.generated.json');

console.log('📝 Generating dynamic CONTRACTS list for TypeScript codegen...');
console.log('🔍 Scanning for contract schemas under contracts/ and vendor/...');

// Collect all IDL JSON files under contracts/**/schema/*.json and vendor/**/schema/*.json
const schemaFiles = [];
function scan(dir) {
  if (!fs.existsSync(dir)) return;
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      scan(fullPath);
    } else if (entry.name.endsWith('.json') && path.dirname(fullPath).endsWith('schema')) {
      // Skip the vendor manifest file
      if (entry.name === 'external-schemas.json') continue;
      schemaFiles.push(fullPath);
    }
  }
}
scan(contractsDir);
scan(vendorDir);
schemaFiles.sort();

// Group files by their schema directory so we can detect multi-IDL dirs
const dirGroups = {};
for (const jsonPath of schemaFiles) {
  const schemaDir = path.dirname(jsonPath);
  if (!dirGroups[schemaDir]) dirGroups[schemaDir] = [];
  dirGroups[schemaDir].push(jsonPath);
}

// For directories with multiple IDL files, create isolated subdirectories so that
// ts-codegen always sees exactly one JSON file per directory (CosmWasm 1.1+ IDL mode).
for (const [schemaDir, files] of Object.entries(dirGroups)) {
  if (files.length > 1) {
    for (const f of files) {
      const contractName = path.basename(f, '.json');
      const isolatedDir = path.join(schemaDir, contractName);
      fs.mkdirSync(isolatedDir, { recursive: true });
      fs.copyFileSync(f, path.join(isolatedDir, path.basename(f)));
    }
  }
}

// Build the CONTRACTS array
const CONTRACTS = [];

for (const jsonPath of schemaFiles) {
  const schemaDir = path.dirname(jsonPath);
  const files = dirGroups[schemaDir];
  const outName = path.basename(jsonPath, '.json');
  const name = toPascalCase(outName);

  // If this dir had multiple IDL files, point to the isolated subdir
  const effectiveSchemaDir = files.length > 1
    ? path.join(schemaDir, outName)
    : schemaDir;

  // Relative path from scripts/ts/ to the effective schema dir
  const relativeDir = path.relative(outputDir, effectiveSchemaDir).replace(/\\/g, '/');

  CONTRACTS.push({
    name,
    dir: relativeDir,
    entryFiles: [
      `${name}.types.ts`,
      `${name}.client.ts`,
      `${name}.message-composer.ts`,
    ],
    globalName: name,
    outName,
  });
}

fs.writeFileSync(outputFile, JSON.stringify(CONTRACTS, null, 2));

console.log(`✅ Generated ${CONTRACTS.length} contract entries → scripts/ts/contracts.generated.json`);
for (const c of CONTRACTS) {
  console.log(`   • ${c.name}  (dir: ${c.dir})`);
}
