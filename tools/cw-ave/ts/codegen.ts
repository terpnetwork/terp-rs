import codegen from "@cosmwasm/ts-codegen";

codegen({
  contracts: [
    {
      name: "CwAve",
      dir: "../contracts/cw-ave/schema",
    },
    {
      name: "CwAveFactory",
      dir: "../contracts/cw-ave-factory/schema",
    }
  ],
  outPath: "./src/",

  // options are completely optional ;)
  options: {
    bundle: {
      bundleFile: "index.ts",
      scope: "contracts",
    },
    types: {
      enabled: true,
    },
    client: {
      enabled: true,
    },
    reactQuery: {
      enabled: true,
      optionalClient: false,
      version: 'v4',
      mutations: false,
      queryKeys: true,
      queryFactory: true,
    },
    recoil: {
      enabled: false,
    },
    messageComposer: {
      enabled: true,
    },
  },
}).then(() => {
  console.log("✨ all done!");
});
