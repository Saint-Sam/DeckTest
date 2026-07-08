#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LEGACY="$ROOT/vendor/legacy-forge"
JAVA_HOME="${JAVA_HOME:-$ROOT/.toolchains/jdks/amazon-corretto-17.jdk/Contents/Home}"
M2="$ROOT/.toolchains/m2"
OUT="$ROOT/target/tmp/legacy-layer-snapshot"

mkdir -p "$OUT/classes"
mkdir -p "$OUT/home"

classpath_entries=(
  "$LEGACY/forge-core/target/classes"
  "$LEGACY/forge-game/target/classes"
  "$LEGACY/forge-ai/target/classes"
  "$LEGACY/forge-gui/target/classes"
  "$LEGACY/forge-gui-desktop/target/classes"
  "$LEGACY/forge-gui-desktop/target/test-classes"
  "$M2/com/google/guava/guava/33.3.1-jre/guava-33.3.1-jre.jar"
  "$M2/org/apache/commons/commons-lang3/3.18.0/commons-lang3-3.18.0.jar"
)

while IFS= read -r jar; do
  classpath_entries+=("$jar")
done < <(find "$M2" -name '*.jar' -type f | sort)

IFS=:
CP="${classpath_entries[*]}"
unset IFS

"$JAVA_HOME/bin/javac" -cp "$CP" -d "$OUT/classes" \
  "$ROOT/tools/legacy-layer-snapshot/src/CpLayersLegacySnapshot.java"

cd "$LEGACY/forge-gui-desktop"
"$JAVA_HOME/bin/java" -Djava.awt.headless=true -Duser.home="$OUT/home" \
  -cp "$OUT/classes:$CP" CpLayersLegacySnapshot "$@"
