#!/usr/bin/env python3
from pathlib import Path


def main() -> None:
    build_gradle_path = Path("src-tauri/gen/android/app/build.gradle.kts")
    if not build_gradle_path.exists():
        raise SystemExit(f"Missing Android Gradle file: {build_gradle_path}")

    build_gradle = build_gradle_path.read_text(encoding="utf-8")

    # Add the generated keystore.properties loader once.
    import_line = "import java.util.Properties"
    if import_line not in build_gradle:
        build_gradle = f"{import_line}\n{build_gradle}"

    # Inject the release signing config into Tauri's generated Android project.
    signing_snippet = """\

val keystorePropertiesFile = rootProject.file("keystore.properties")
val keystoreProperties = Properties()

if (keystorePropertiesFile.exists()) {
    keystoreProperties.load(keystorePropertiesFile.inputStream())
}

android {
    signingConfigs {
        create("release") {
            keyAlias = keystoreProperties["keyAlias"] as String
            keyPassword = keystoreProperties["keyPassword"] as String
            storeFile = rootProject.file(keystoreProperties["storeFile"] as String)
            storePassword = keystoreProperties["storePassword"] as String
        }
    }
}
"""
    if 'keystorePropertiesFile = rootProject.file("keystore.properties")' not in build_gradle:
        marker = "android {"
        index = build_gradle.find(marker)
        if index == -1:
            raise SystemExit("Unable to find android block in Android build.gradle.kts")

        build_gradle = build_gradle[:index] + signing_snippet + "\n" + build_gradle[index:]

    # Attach the signing config to whichever release buildType shape Gradle generated.
    release_signing_line = '            signingConfig = signingConfigs.getByName("release")'
    if release_signing_line not in build_gradle:
        for release_marker in (
            '        getByName("release") {',
            '        named("release") {',
            "        release {",
        ):
            if release_marker in build_gradle:
                replacement = f"{release_marker}\n{release_signing_line}"
                build_gradle = build_gradle.replace(release_marker, replacement, 1)
                break
        else:
            raise SystemExit("Unable to find release buildType block in Android build.gradle.kts")

    build_gradle_path.write_text(build_gradle, encoding="utf-8")


if __name__ == "__main__":
    main()
