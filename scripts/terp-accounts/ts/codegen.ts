import codegen from "@cosmwasm/ts-codegen";
import { build, BuildOptions } from 'esbuild';
import path from 'path';

const OUT_DIR = './src';
const DIST_DIR = './dist';
const ESBUILD_EXTERNAL = [
  '@interchainjs/*',
  'interchainjs',
  'interchainjs/*',
  '@chain-registry/*',
];

const ESBUILD_SHARED: Partial<BuildOptions> = {
  bundle: true,
  minify: true,
  sourcemap: true,
  target: ['es2020'],
  external: ESBUILD_EXTERNAL,
};


const CONTRACTS = [
  {
    name: "AccountMinter",
    dir: "../../contracts/terp721-account-manifold/schema",
    entryFiles: ['AccountMinter.types.ts', 'AccountMinter.client.ts', 'AccountMinter.message-composer.ts'],
    globalName: 'AccountMinter',
    outName: 'account-minter',

  },
  {
    name: "Terp721Account",
    dir: "../../contracts/terp721-account/schema",
    entryFiles: ['Terp721Account.types.ts', 'Terp721Account.client.ts', 'Terp721Account.message-composer.ts'],
    globalName: 'Terp721Account',
    outName: 'terp721-account',
  },
]
async function main() {

  await codegen({
    contracts: CONTRACTS.map(c => ({ name: c.name, dir: c.dir })),
    outPath: "./src/",
    options: {
      bundle: {
        bundleFile: "bundle.ts",
        scope: "contracts",
      },
      types: {
        enabled: true,
      },
      client: {
        enabled: true,
      },
      reactQuery: {
        enabled: false,
        optionalClient: true,
        version: "v4",
        mutations: true,
        queryKeys: true,
      },
      recoil: {
        enabled: false,
      },
      messageComposer: {
        enabled: true,
      },
    },
  });

  console.log('✨ codegen done — bundling for browser...\n');
  // 2. Build per-contract bundles (IIFE + ESM)
  //    Create a barrel entry per contract so esbuild gets a single input → single output
  const fs = await import('fs');
  fs.mkdirSync(path.join(OUT_DIR, '_entry'), { recursive: true });

  for (const contract of CONTRACTS) {
    // Write a tiny barrel that re-exports all files for this contract
    const barrel = contract.entryFiles
      .map(f => `export * from '../${f.replace('.ts', '')}';`)
      .join('\n');
    const barrelPath = path.join(OUT_DIR, '_entry', `${contract.outName}.ts`);
    fs.writeFileSync(barrelPath, barrel);

    await Promise.all([
      build({
        ...ESBUILD_SHARED,
        entryPoints: [barrelPath],
        format: 'iife',
        globalName: contract.globalName,
        outfile: path.join(DIST_DIR, `${contract.outName}.js`),
      }),
      build({
        ...ESBUILD_SHARED,
        entryPoints: [barrelPath],
        format: 'esm',
        outfile: path.join(DIST_DIR, 'esm', `${contract.outName}.js`),
      }),
    ]);

    console.log(`  📦 ${contract.outName}.js  (IIFE + ESM)`);
  }

  // 3. Build the combined "all contracts" bundle from the barrel file
  await Promise.all([
    build({
      ...ESBUILD_SHARED,
      entryPoints: [path.join(OUT_DIR, 'bundle.ts')],
      format: 'iife',
      globalName: 'CwContracts',
      outfile: path.join(DIST_DIR, 'contracts.js'),
    }),
    build({
      ...ESBUILD_SHARED,
      entryPoints: [path.join(OUT_DIR, 'bundle.ts')],
      format: 'esm',
      outfile: path.join(DIST_DIR, 'contracts.esm.js'),
    }),
  ]);

  console.log(`  📦 contracts.js           (all-in-one IIFE + ESM)`);
  console.log('\n✅ dist/ ready');
  console.log('   Per-contract:  dist/<name>.js           → <script src="...">');
  console.log('   Per-contract:  dist/esm/<Name>.*.js     → <script type="module">');
  console.log('   All-in-one:    dist/contracts.js        → window.CwContracts.contracts.*');
  console.log('   All-in-one:    dist/contracts.esm.js    → import { contracts } from ...');
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
