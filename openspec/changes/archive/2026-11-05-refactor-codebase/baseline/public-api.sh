#!/usr/bin/env bash
# Usage: public-api.sh  — prints the pre-refactor public surface, one path per line.
#  * walks `pub mod x;` declarations from src/lib.rs recursively (pub(crate)/private mods excluded;
#    layout src/<m>.rs | src/<m>/mod.rs | src/<parent>/<m>.rs)
#  * emits `pub` fn/struct/enum/trait/const/static/type/union items at any indentation, except inside
#    `impl` blocks and `#[cfg(test)]` items; enum variants are not listed
#  * emits `pub fn` of inherent `impl <Type>` blocks as iroh_tunnel::<mod>::<Type>::<method>
#  * unions with the `iroh_tunnel::…` paths used under tests/ and examples/
# Heuristic (line/brace based; inline `pub mod x { }` items are attributed to the file's module).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

items() { # items <file> <modpath>
  awk -v mp="$2" '
    function clean(s) { gsub(/\/\/.*/, "", s); gsub(/"([^"\\]|\\.)*"/, "\"\"", s); gsub(/\047.\047/, "", s); return s }
    BEGIN { depth = 0; mode = ""; pend = ""; testpend = 0 }
    {
      line = clean($0)
      o = line; c = line
      nopen = gsub(/\{/, "", o); nclose = gsub(/\}/, "", c)
      if (mode == "" && pend == "") {
        if (line ~ /^[ \t]*#\[cfg\(test\)\]/) { testpend = 1 }
        else if (line ~ /^[ \t]*#\[/) { }
        else if (testpend) { testpend = 0; pend = "skip"; hdr = "" }
        else if (line ~ /^[ \t]*(unsafe[ \t]+)?impl[ \t<]/) { pend = "impl"; hdr = "" }
      }
      if (pend != "") {
        hdr = hdr " " line
        if (nopen == 0) { if (line ~ /;/) pend = ""; }
        else {
          if (pend == "skip") mode = "skip"
          else if (hdr ~ /[ \t>]for[ \t]/) mode = "skip"
          else {
            h = hdr; sub(/^[ \t]*(unsafe[ \t]+)?impl/, "", h)
            while (h ~ /^[ \t]*</) { # strip generic params
              d = 0; k = 0
              for (n = 1; n <= length(h); n++) { ch = substr(h, n, 1); if (ch == "<") d++; else if (ch == ">") { d--; if (d == 0) { k = n; break } } }
              h = substr(h, k + 1)
            }
            sub(/^[ \t]+/, "", h); sub(/^(::)?([a-z_]+::)*/, "", h)
            match(h, /^[A-Za-z0-9_]+/); ty = substr(h, RSTART, RLENGTH)
            mode = "impl"
          }
          base = depth; pend = ""
        }
      }
      else if (mode == "impl") {
        if (depth == base + 1 && match(line, /^[ \t]*pub[ \t]+(async[ \t]+|const[ \t]+|unsafe[ \t]+)*fn[ \t]+[A-Za-z0-9_]+/)) {
          s = substr(line, RSTART, RLENGTH); sub(/.*fn[ \t]+/, "", s); print mp "::" ty "::" s
        }
      }
      else if (mode == "") {
        if (match(line, /^[ \t]*pub[ \t]+(async[ \t]+|const[ \t]+|unsafe[ \t]+)*(fn|struct|enum|trait|const|static|type|union)[ \t]+[A-Za-z0-9_]+/)) {
          s = substr(line, RSTART, RLENGTH); sub(/^.*[ \t]/, "", s); print mp "::" s
        }
      }
      depth += nopen - nclose
      if (mode != "" && depth <= base) mode = ""
    }' "$1"
}

walk() { # walk <file> <modpath> <dir containing child module files>
  local file="$1" mp="$2" dir="$3" m child
  echo "$mp"
  [ "$mp" = iroh_tunnel ] || items "$file" "$mp"
  for m in $(grep -E '^[[:space:]]*pub[[:space:]]+mod[[:space:]]+[A-Za-z0-9_]+[[:space:]]*;' "$file" \
      | sed -E 's/.*mod[[:space:]]+([A-Za-z0-9_]+).*/\1/'); do
    if [ -f "$dir/$m.rs" ]; then walk "$dir/$m.rs" "$mp::$m" "$dir/$m"
    elif [ -f "$dir/$m/mod.rs" ]; then walk "$dir/$m/mod.rs" "$mp::$m" "$dir/$m"
    else echo "public-api.sh: missing module file for $mp::$m" >&2; exit 1; fi
  done
}

used() {
  find tests examples -name '*.rs' 2>/dev/null | sort | while read -r f; do tr '\n' ' ' < "$f"; echo; done | awk '
    function skipws() { while (substr(t, i, 1) ~ /[ \t]/) i++ }
    function emit(p) { sub(/::$/, "", p); if (p ~ /^iroh_tunnel::[A-Za-z0-9_]/) print p }
    function seg(prefix,   name, st) {
      skipws(); st = i
      while (substr(t, i, 1) ~ /[A-Za-z0-9_]/) i++
      name = substr(t, st, i - st); skipws()
      if (name == "self") { emit(prefix); return }
      if (substr(t, i, 2) == "::") {
        i += 2; skipws()
        if (substr(t, i, 1) == "{") {
          i++
          while (i <= length(t)) {
            seg(prefix name "::"); skipws()
            if (substr(t, i, 1) == ",") { i++; skipws(); if (substr(t, i, 1) == "}") { i++; break }; continue }
            if (substr(t, i, 1) == "}") i++
            break
          }
        } else if (substr(t, i, 1) == "*") { i++; emit(prefix name) }
        else seg(prefix name "::")
      } else if (name != "") emit(prefix name)
    }
    { t = $0; pos = 1
      while ((k = index(substr(t, pos), "iroh_tunnel::")) > 0) {
        i = pos + k - 1 + 13; pos = i; seg("iroh_tunnel::")
        if (i > pos) pos = i
      } }'
}

{ walk src/lib.rs iroh_tunnel src; used; } | sort -u
