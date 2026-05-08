const release = process.argv.includes("--release");

const entries = [
  { entrypoint: "ui/src/app/index.ts", outfile: "ui/app.js" },
  { entrypoint: "ui/src/settings/index.ts", outfile: "ui/settings.js" },
];

for (const { entrypoint, outfile } of entries) {
  const result = await Bun.build({
    entrypoints: [entrypoint],
    outdir: ".",
    naming: outfile,
    format: "iife",
    minify: release,
  });
  if (!result.success) {
    console.error(`Failed to build ${entrypoint}:`);
    for (const log of result.logs) console.error(log);
    process.exit(1);
  }
}
