import { cp, mkdir, rm } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const mobile = path.resolve(desktop, "../mobile");
const npmCli = process.env.npm_execpath;
if (!npmCli) throw new Error("npm_execpath is unavailable");
const run = (args, cwd) => { const result=spawnSync(process.execPath,[npmCli,...args],{cwd,stdio:"inherit"}); if(result.error)throw result.error; if(result.status!==0)process.exit(result.status??1); };
run(["run","build:web"],desktop);
run(["run","build"],mobile);
const target=path.join(desktop,"dist/mobile");
await rm(target,{recursive:true,force:true});
await mkdir(target,{recursive:true});
await cp(path.join(mobile,"dist"),target,{recursive:true});
