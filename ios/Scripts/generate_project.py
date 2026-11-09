#!/usr/bin/env python3
"""Generate a standalone Xcode project; no xcodegen or desktop Package.swift needed."""
from pathlib import Path
import hashlib
import json
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
OBJECTS = {}

def ident(key):
    return hashlib.sha256(key.encode()).hexdigest()[:24].upper()

def quote(value):
    return json.dumps(str(value), ensure_ascii=False)

def encode(value, depth=0):
    if isinstance(value, dict):
        return "{\n" + "\n".join("\t" * (depth + 1) + quote(k) + " = " + encode(v, depth + 1) + ";" for k, v in value.items()) + "\n" + "\t" * depth + "}"
    if isinstance(value, list):
        return "(" + ", ".join(encode(item, depth + 1) for item in value) + ")"
    return quote(value)

def obj(key, isa, **values):
    token = ident(key)
    OBJECTS[token] = {"isa": isa, **values}
    return token

APP_FILES = sorted((ROOT / "App").glob("*.swift"))
SHARED_FILES = sorted((ROOT / "Shared").glob("*.swift"))
PARSER = [ROOT.parent / "Sources/ProxyLogic/Subscription.swift", ROOT.parent / "Sources/ProxyLogic/TrafficMode.swift"]
refs = {}
for path in APP_FILES + SHARED_FILES + PARSER + list((ROOT / "PacketTunnel").glob("*.swift")) + list((ROOT / "Tests").glob("*.swift")):
    relative = str(path.relative_to(ROOT)) if path.is_relative_to(ROOT) else "../" + str(path.relative_to(ROOT.parent))
    relative = relative.replace("\\", "/")
    refs[path] = obj("file:" + relative, "PBXFileReference", lastKnownFileType="sourcecode.swift", path=relative, sourceTree="<group>")
configs = []
for path in sorted((ROOT / "Config").glob("*")):
    configs.append(obj("config:" + path.name, "PBXFileReference", lastKnownFileType="text.plist.xml", path="Config/" + path.name, sourceTree="<group>"))
package = obj("yams", "XCRemoteSwiftPackageReference", repositoryURL="https://github.com/jpsim/Yams.git", requirement={"kind": "exactVersion", "version": "6.2.2"})
project_id = ident("project")
products = []
targets = []

def configuration_list(key, base):
    items = []
    for name in ["Debug", "Release"]:
        settings = dict(base)
        settings.update({"SWIFT_OPTIMIZATION_LEVEL": "-Onone" if name == "Debug" else "-O", "DEBUG_INFORMATION_FORMAT": "dwarf" if name == "Debug" else "dwarf-with-dsym"})
        if name == "Debug":
            settings["SWIFT_ACTIVE_COMPILATION_CONDITIONS"] = "$(inherited) DEBUG " + settings.get("SWIFT_ACTIVE_COMPILATION_CONDITIONS", "")
            settings["ENABLE_TESTABILITY"] = "YES"
        items.append(obj(key + ":" + name, "XCBuildConfiguration", name=name, buildSettings=settings))
    return obj(key + ":config-list", "XCConfigurationList", buildConfigurations=items, defaultConfigurationIsVisible="0", defaultConfigurationName="Release")

common = {"SDKROOT": "iphoneos", "IPHONEOS_DEPLOYMENT_TARGET": "17.0", "SWIFT_VERSION": "5.0", "TARGETED_DEVICE_FAMILY": "1,2", "CODE_SIGN_STYLE": "Automatic", "DEVELOPMENT_TEAM": "", "SUPPORTED_PLATFORMS": "iphoneos iphonesimulator", "SUPPORTS_MACCATALYST": "NO", "CLANG_ENABLE_MODULES": "YES", "ENABLE_USER_SCRIPT_SANDBOXING": "YES"}

def target(name, kind, source_paths, module, bundle, extra=None, yams=False, dependencies=None, embedded=None):
    settings = dict(common)
    settings.update({"PRODUCT_NAME": name, "PRODUCT_MODULE_NAME": module, "PRODUCT_BUNDLE_IDENTIFIER": bundle, "MARKETING_VERSION": "0.1.0", "CURRENT_PROJECT_VERSION": "1", "LD_RUNPATH_SEARCH_PATHS": ["$(inherited)", "@executable_path/Frameworks"]})
    settings.update(extra or {})
    product_type = {"app": "com.apple.product-type.application", "extension": "com.apple.product-type.app-extension", "test": "com.apple.product-type.bundle.unit-test"}[kind]
    suffix, file_type = {"app": (".app", "wrapper.application"), "extension": (".appex", "wrapper.app-extension"), "test": (".xctest", "wrapper.cfbundle")}[kind]
    product = obj(name + ":product", "PBXFileReference", explicitFileType=file_type, includeInIndex="0", path=name + suffix, sourceTree="BUILT_PRODUCTS_DIR")
    products.append(product)
    source_builds = [obj(name + ":source:" + str(path), "PBXBuildFile", fileRef=refs[path]) for path in source_paths]
    source_phase = obj(name + ":sources", "PBXSourcesBuildPhase", buildActionMask="2147483647", files=source_builds, runOnlyForDeploymentPostprocessing="0")
    links, package_products = [], []
    if yams:
        dep = obj(name + ":yams", "XCSwiftPackageProductDependency", package=package, productName="Yams")
        links.append(obj(name + ":yams-build", "PBXBuildFile", productRef=dep))
        package_products.append(dep)
    phases = [source_phase, obj(name + ":frameworks", "PBXFrameworksBuildPhase", buildActionMask="2147483647", files=links, runOnlyForDeploymentPostprocessing="0"), obj(name + ":resources", "PBXResourcesBuildPhase", buildActionMask="2147483647", files=[], runOnlyForDeploymentPostprocessing="0")]
    deps = []
    for dependency in dependencies or []:
        proxy = obj(name + ":proxy:" + dependency, "PBXContainerItemProxy", containerPortal=project_id, proxyType="1", remoteGlobalIDString=ident("target:" + dependency), remoteInfo=dependency)
        deps.append(obj(name + ":dep:" + dependency, "PBXTargetDependency", target=ident("target:" + dependency), targetProxy=proxy))
    if embedded:
        build = obj(name + ":embed", "PBXBuildFile", fileRef=ident(embedded + ":product"), settings={"ATTRIBUTES": ["RemoveHeadersOnCopy"]})
        phases.append(obj(name + ":embed-phase", "PBXCopyFilesBuildPhase", buildActionMask="2147483647", dstPath="", dstSubfolderSpec="13", files=[build], name="Embed App Extensions", runOnlyForDeploymentPostprocessing="0"))
    token = obj("target:" + name, "PBXNativeTarget", name=name, productName=name, productReference=product, productType=product_type, buildConfigurationList=configuration_list(name, settings), buildPhases=phases, buildRules=[], dependencies=deps, packageProductDependencies=package_products)
    targets.append(token)
    return token

manager = target("SmartClashManager", "app", APP_FILES + SHARED_FILES + PARSER, "SmartClashManager", "com.mrtager.smartclash.manager", {"INFOPLIST_FILE": "Config/App.Info.plist", "GENERATE_INFOPLIST_FILE": "NO"}, yams=True)
tunnel = target("SmartClashPacketTunnel", "extension", list((ROOT / "PacketTunnel").glob("*.swift")), "SmartClashPacketTunnel", "com.mrtager.smartclash.ios.PacketTunnel", {"INFOPLIST_FILE": "Config/PacketTunnel.Info.plist", "GENERATE_INFOPLIST_FILE": "NO", "CODE_SIGN_ENTITLEMENTS": "Config/VPN.entitlements", "APPLICATION_EXTENSION_API_ONLY": "YES", "SKIP_INSTALL": "YES", "LD_RUNPATH_SEARCH_PATHS": ["$(inherited)", "@executable_path/../../Frameworks"]})
vpn = target("SmartClashVPN", "app", APP_FILES + SHARED_FILES + PARSER, "SmartClashVPN", "com.mrtager.smartclash.ios", {"INFOPLIST_FILE": "Config/App.Info.plist", "GENERATE_INFOPLIST_FILE": "NO", "CODE_SIGN_ENTITLEMENTS": "Config/VPN.entitlements", "SWIFT_ACTIVE_COMPILATION_CONDITIONS": "SMARTCLASH_TUNNEL"}, yams=True, dependencies=["SmartClashPacketTunnel"], embedded="SmartClashPacketTunnel")
tests = target("SmartClashTests", "test", list((ROOT / "Tests").glob("*.swift")), "SmartClashMobileTests", "com.mrtager.smartclash.manager.tests", {"GENERATE_INFOPLIST_FILE": "YES", "TEST_HOST": "$(BUILT_PRODUCTS_DIR)/SmartClashManager.app/SmartClashManager", "BUNDLE_LOADER": "$(TEST_HOST)"}, dependencies=["SmartClashManager"])
product_group = obj("products", "PBXGroup", name="Products", children=products, sourceTree="<group>")
main_group = obj("main-group", "PBXGroup", children=list(refs.values()) + configs + [product_group], sourceTree="<group>")
obj("project", "PBXProject", attributes={"LastUpgradeCheck": "1600", "BuildIndependentTargetsInParallel": "YES"}, buildConfigurationList=configuration_list("project", {"CLANG_ENABLE_OBJC_ARC": "YES", "CLANG_ENABLE_MODULES": "YES", "SWIFT_VERSION": "5.0", "IPHONEOS_DEPLOYMENT_TARGET": "17.0", "SWIFT_STRICT_CONCURRENCY": "targeted"}), compatibilityVersion="Xcode 14.0", developmentRegion="zh_CN", hasScannedForEncodings="0", knownRegions=["zh_CN", "en", "Base"], mainGroup=main_group, productRefGroup=product_group, projectDirPath="", projectRoot="", targets=targets, packageReferences=[package])
output = {"archiveVersion": "1", "classes": {}, "objectVersion": "56", "objects": OBJECTS, "rootObject": project_id}
(ROOT / "SmartClash.xcodeproj/project.pbxproj").write_text("// !$*UTF8*$!\n" + encode(output) + "\n", encoding="utf-8")

def scheme(name, app_name, testable=False):
    root = ET.Element("Scheme", LastUpgradeVersion="1600", version="1.3")
    action = ET.SubElement(root, "BuildAction", parallelizeBuildables="YES", buildImplicitDependencies="YES")
    entries = ET.SubElement(action, "BuildActionEntries")
    entry = ET.SubElement(entries, "BuildActionEntry", buildForTesting="YES", buildForRunning="YES", buildForProfiling="YES", buildForArchiving="YES", buildForAnalyzing="YES")
    def reference(parent, item, suffix):
        ET.SubElement(parent, "BuildableReference", BuildableIdentifier="primary", BlueprintIdentifier=ident("target:" + item), BuildableName=item + suffix, BlueprintName=item, ReferencedContainer="container:SmartClash.xcodeproj")
    reference(entry, app_name, ".app")
    test = ET.SubElement(root, "TestAction", buildConfiguration="Debug", selectedDebuggerIdentifier="Xcode.DebuggerFoundation.Debugger.LLDB", selectedLauncherIdentifier="Xcode.IDEFoundation.Launcher.LLDB", shouldUseLaunchSchemeArgsEnv="YES")
    if testable:
        item = ET.SubElement(ET.SubElement(test, "Testables"), "TestableReference", skipped="NO")
        reference(item, "SmartClashTests", ".xctest")
    run = ET.SubElement(root, "LaunchAction", buildConfiguration="Debug", selectedDebuggerIdentifier="Xcode.DebuggerFoundation.Debugger.LLDB", selectedLauncherIdentifier="Xcode.IDEFoundation.Launcher.LLDB", launchStyle="0", useCustomWorkingDirectory="NO", ignoresPersistentStateOnLaunch="NO", debugDocumentVersioning="YES", allowLocationSimulation="YES")
    reference(ET.SubElement(run, "BuildableProductRunnable", runnableDebuggingMode="0"), app_name, ".app")
    profile = ET.SubElement(root, "ProfileAction", buildConfiguration="Release", shouldUseLaunchSchemeArgsEnv="YES", savedToolIdentifier="", useCustomWorkingDirectory="NO", debugDocumentVersioning="YES")
    reference(ET.SubElement(profile, "BuildableProductRunnable", runnableDebuggingMode="0"), app_name, ".app")
    ET.SubElement(root, "AnalyzeAction", buildConfiguration="Debug")
    ET.SubElement(root, "ArchiveAction", buildConfiguration="Release", revealArchiveInOrganizer="YES")
    ET.indent(root)
    ET.ElementTree(root).write(ROOT / ("SmartClash.xcodeproj/xcshareddata/xcschemes/" + name + ".xcscheme"), encoding="utf-8", xml_declaration=True)

scheme("SmartClash-Manager", "SmartClashManager", True)
scheme("SmartClash-VPN", "SmartClashVPN")
print("Generated standalone iOS project with manager, guarded extension and test targets.")
