function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * 判断 Cargo.lock 中指定包是否来自本地 path。
 *
 * GitHub Windows runner 会把工作树文本签出为 CRLF；锁文件语义不能依赖宿主换行符。
 */
export function isVendoredPathPackage(cargoLock, name, version) {
  const packageBody = lockedPackageBody(cargoLock, name, version);
  return Boolean(packageBody && !/^(source|checksum) =/m.test(packageBody));
}

/** 统一换行后返回包区块，供需要继续核验 source/checksum 的调用方使用。 */
export function lockedPackageBody(cargoLock, name, version) {
  const normalized = cargoLock.replace(/\r\n?/g, "\n");
  const packageBlock = normalized.match(
    new RegExp(
      `\\[\\[package\\]\\]\\nname = "${escapeRegExp(name)}"\\nversion = "${escapeRegExp(version)}"\\n([\\s\\S]*?)(?=\\n\\[\\[package\\]\\]|$)`,
    ),
  );
  return packageBlock?.[1] ?? null;
}
