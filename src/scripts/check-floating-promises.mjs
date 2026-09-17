#!/usr/bin/env node

import ts from "typescript";
import { dirname, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const sourceRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const configPath = resolve(sourceRoot, "tsconfig.js.json");

function loadProgram() {
  const loaded = ts.readConfigFile(configPath, ts.sys.readFile);
  if (loaded.error) throw new Error(ts.flattenDiagnosticMessageText(loaded.error.messageText, "\n"));
  const parsed = ts.parseJsonConfigFileContent(loaded.config, ts.sys, sourceRoot);
  if (parsed.errors.length > 0) {
    throw new Error(parsed.errors.map(error => ts.flattenDiagnosticMessageText(error.messageText, "\n")).join("\n"));
  }
  return ts.createProgram(parsed.fileNames, parsed.options);
}

function isTarget(sourceFile) {
  const path = relative(sourceRoot, sourceFile.fileName).split(sep).join("/");
  return path.startsWith("js/preview/") || path.startsWith("js/settings/");
}

function hasRejectionHandler(expression) {
  if (!ts.isCallExpression(expression) || !ts.isPropertyAccessExpression(expression.expression)) return false;
  const name = expression.expression.name.text;
  if (name === "catch") return true;
  if (name === "then" && expression.arguments.length >= 2) return true;
  return hasRejectionHandler(expression.expression.expression);
}

function formatLocation(sourceFile, node) {
  const start = sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile));
  return `${relative(sourceRoot, sourceFile.fileName)}:${start.line + 1}:${start.character + 1}`;
}

export function findFloatingPromises(program, include = isTarget) {
  const checker = program.getTypeChecker();
  const failures = [];
  for (const sourceFile of program.getSourceFiles().filter(include)) {
    const visit = node => {
      if (ts.isExpressionStatement(node) && ts.isCallExpression(node.expression)) {
        const promised = checker.getPromisedTypeOfPromise(checker.getTypeAtLocation(node.expression));
        if (promised && !hasRejectionHandler(node.expression)) {
          failures.push(`${formatLocation(sourceFile, node.expression)} 未处理 Promise；使用 await、return、void 或 catch`);
        }
      }
      ts.forEachChild(node, visit);
    };
    visit(sourceFile);
  }
  return failures;
}

const invokedAsScript = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedAsScript) {
  try {
    const failures = findFloatingPromises(loadProgram());
    if (failures.length > 0) {
      for (const failure of failures) console.error(`Frontend static error: ${failure}`);
      process.exitCode = 1;
    } else {
      console.log("Frontend Promise check passed: preview and settings");
    }
  } catch (error) {
    console.error(`Frontend static error: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
}
