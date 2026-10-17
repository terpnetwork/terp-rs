---
title: Placeholder dictionary
description: Leaning Tower of Babel — Universal Layer v1 grammar and Collection-Layer dicts for CW-SVG templates.
---

# CW-SVG Placeholder Dictionary Specification

*Leaning Tower of Babel — Universal Layer v1*

## Overview

The Placeholder Dictionary is a two-layer naming system for SVG template variables:

1. **Universal Layer** (this document) — stable grammar and type vocabulary shared across all collections.
2. **Collection Layer** — a `*.dict.json` file per SVG that specifies exact parameter ranges, palette groups, and generator notes for that collection's variables.

The Universal Layer never changes. Collection Layers are unique to each creator and SVG. LLMs are prompted with the Universal Layer and the target SVG to produce the Collection Layer.

---

## Variable Name Grammar

Variable names follow a hyphen-separated structure:

```
varname    ::= class ( "-" qualifier )*
class      ::= semantic-prefix
qualifier  ::= instance | role-suffix
instance   ::= letter | digit+
role-suffix ::= "base" | "lo" | "hi" | "fast" | "slow" | label
label      ::= [a-z][a-z0-9]*
```

The **class** (first hyphen-delimited segment) determines the implied `VariableKind`. **Qualifiers** narrow context or declare group membership.

---

## Semantic Class Vocabulary

| Class Prefix | Implied Kind | Canonical Default Range | Notes |
|---|---|---|---|
| `color` | `RgbStyled` | palette-defined | fill/stroke colors |
| `hue` | `RgbStyled` | hue-anchored palette | hue-dominant variant |
| `glow` | `RgbStyled` | same palette as `color` group | filter/shadow colors |
| `opacity`, `alpha` | `Range` | [0.0, 1.0] prec 2 | unitless |
| `pulse`, `breath` | `Range` | [1.2, 4.5] prec 2 | slow animation (template appends `s`) |
| `flicker`, `strobe` | `Range` | [0.08, 0.50] prec 2 | fast animation (template appends `s`) |
| `drift`, `float` | `Range` | [3.0, 15.0] prec 1 | very slow motion (template appends `s`) |
| `rotate`, `spin` | `Range` | [0, 360] prec 0 | degrees, unitless |
| `scale`, `size` | `Range` | [0.5, 2.0] prec 2 | unitless multiplier |
| `blur` | `Range` | [0.5, 8.0] prec 1 | unitless px |
| `offset`, `shift` | `Range` | context-defined | displacement |
| `toggle` | `Options` | `["0", "1"]` | binary switch |
| `mode` | `Options` | collection-defined | multi-state selector |
| `val` | `Range` | context-defined | generic scalar |

### Unit Handling Convention

Timing classes (`pulse`, `flicker`, `drift`, etc.) produce **unitless decimals**. The SVG template appends the CSS unit:

```xml
<!-- Correct: unit lives outside the placeholder -->
style="--pulse-dur:${pulse-a}s"

<!-- Incorrect: unit embedded in placeholder value (Range can't produce "2.42s") -->
style="--pulse-dur:${pulse-a}"
```

This keeps `VariableKind::Range` values as clean numbers and allows the same variable to appear in different unit contexts.

---

## Palette Groups

Variables sharing the same `class` and group qualifier form a **palette group**. All members are assigned the same `RgbStyled` ranges, ensuring visual coherence within a collection.

```
color-a, color-b, ..., color-m  → same palette group
glow-color                      → anchored to that same palette
```

An **anchor** variable (e.g., `glow-color`, `color-base`) sets the palette origin. All other group members inherit the same ranges. Group membership is declared in the Collection Layer dictionary.

---

## Collection Layer Dictionary Format

```json
{
  "version": "1",
  "collection": "my-collection",
  "description": "Human-readable description of this SVG's visual intent",

  "palette_groups": [
    {
      "id": "orb",
      "note": "Neon orb fill colors — four neon hue bands",
      "anchor": "glow-color",
      "members": ["color-a", "color-b", "color-c"],
      "ranges": [
        { "r_min": 50,  "r_max": 120, "g_min": 220, "g_max": 255, "b_min": 50,  "b_max": 120 },
        { "r_min": 0,   "r_max": 60,  "g_min": 200, "g_max": 255, "b_min": 200, "b_max": 255 }
      ]
    }
  ],

  "motion_groups": [
    {
      "id": "breath",
      "note": "Slow neon breath pulse, independent per-orb",
      "members": ["pulse-a", "pulse-b"],
      "min": "1.2",
      "max": "4.5",
      "precision": 2
    },
    {
      "id": "strobe",
      "note": "Fast neon strobe flicker, independent per-orb",
      "members": ["flicker-a", "flicker-b"],
      "min": "0.08",
      "max": "0.50",
      "precision": 2
    }
  ],

  "vars": [
    { "name": "glow-color", "kind": { "rgb_styled": [
        { "r_min": 50, "r_max": 120, "g_min": 220, "g_max": 255, "b_min": 50, "b_max": 120 }
    ]}},
    { "name": "color-a",   "kind": { "rgb_styled": [
        { "r_min": 50, "r_max": 120, "g_min": 220, "g_max": 255, "b_min": 50, "b_max": 120 }
    ]}},
    { "name": "pulse-a",   "kind": { "range": { "min": "1.2", "max": "4.5", "precision": 2 } } },
    { "name": "flicker-a", "kind": { "range": { "min": "0.08", "max": "0.50", "precision": 2 } } }
  ]
}
```

The `vars` field is a `Vec<VariableDef>` array directly compatible with `--vars-json`. All other fields are documentation consumed by humans and LLMs.

---

## LLM Prompting Contract

When generating a Collection Layer dictionary, the LLM receives:

1. This specification (Universal Layer)
2. The SVG template content
3. The list of discovered variable names with occurrence counts

The LLM outputs a valid Collection Layer dictionary JSON. The `vars` array must contain exactly one `VariableDef` for every variable in the template, with `VariableKind` values consistent with the name grammar above.

**Palette coherence rule:** variables in the same palette group must share identical `RgbRange` entries in their `RgbStyled` kind.

**Timing unit rule:** timing variables (`pulse-*`, `flicker-*`, `drift-*`) use `VariableKind::Range` with unitless decimals. The SVG template must have the unit (`s`) appended immediately after the closing `}` of the placeholder.

---

## Entropy Gradients

For palette groups, the Collection Layer can specify an **entropy gradient** that controls how much variation members are allowed relative to the anchor. This is advisory — tooling and LLMs use it to tune the width of `RgbRange` windows:

| Gradient | Description | Approximate channel spread |
|---|---|---|
| `tight` | Nearly identical shades | ±10 per channel |
| `low` | Subtle variation | ±25 per channel |
| `medium` | Noticeable diversity | ±50 per channel |
| `high` | Wide variation within hue band | ±80 per channel |
| `full` | Unconstrained random (use `Rgb`) | full 0–255 |

This gradient vocabulary is advisory for LLMs generating collection dictionaries — it is not enforced on-chain.

---

## Quick Reference: Name → Kind Inference

```
color-*       → RgbStyled (use palette_groups ranges)
glow-*        → RgbStyled (same palette as color group)
hue-*         → RgbStyled (hue-anchored ranges)
opacity-*     → Range [0.0, 1.0] prec 2
alpha-*       → Range [0.0, 1.0] prec 2
pulse-*       → Range [1.2, 4.5] prec 2  (unit outside placeholder)
breath-*      → Range [1.2, 4.5] prec 2  (unit outside placeholder)
flicker-*     → Range [0.08, 0.50] prec 2 (unit outside placeholder)
strobe-*      → Range [0.08, 0.50] prec 2 (unit outside placeholder)
drift-*       → Range [3.0, 15.0] prec 1  (unit outside placeholder)
float-*       → Range [3.0, 15.0] prec 1  (unit outside placeholder)
rotate-*      → Range [0, 360] prec 0
spin-*        → Range [0, 360] prec 0
scale-*       → Range [0.5, 2.0] prec 2
size-*        → Range [0.5, 2.0] prec 2
blur-*        → Range [0.5, 8.0] prec 1
toggle-*      → Options ["0", "1"]
mode-*        → Options (collection-defined list)
val-*         → Range (context-defined)
```
