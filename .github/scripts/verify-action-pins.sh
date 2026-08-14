#!/usr/bin/env bash
# Verify every action pin in the workflows.
#
# Three properties, all mechanical:
#
#   1. Every `uses:` reference is a full 40-hex commit SHA. A tag or branch
#      reference is the thing pinning exists to prevent.
#   2. The SHA is what the version named in the trailing comment resolves to
#      upstream. The comment is decorative to GitHub — it resolves the SHA and
#      never reads the comment — so a pin reading `# v3.0.2` while pointing at
#      an unrelated commit is indistinguishable from a correct one by eye, and
#      that is the exact shape of an attack on a consumer who has pinned.
#   3. One action is not pinned at two different SHAs across the workflows.
#      Each pin can match its own comment while two jobs run different
#      versions, so a per-pin check passes both.
#
# Annotated tags are dereferenced before comparing: `git/ref/tags/<tag>` yields
# a tag OBJECT for those, not a commit, so comparing without following
# `git/tags/<sha>` reports every correct pin as a mismatch. A check that fails
# on correct input gets muted, which is worse than no check.
#
# Usage: verify-action-pins.sh [workflow-dir]      (default .github/workflows)
#
# Upstream lookups all go through api(), the one network seam. Set
# PIN_API_FIXTURES=<dir> to answer them from files instead, so the tests run
# offline: a request for API path `a/b/c` reads `<dir>/a__b__c.json`, and a
# missing file is a 404.

set -uo pipefail

WORKFLOW_DIR="${1:-.github/workflows}"

failures=0
pins=0

fail() {
    printf 'FAIL %s\n' "$*" >&2
    failures=$((failures + 1))
}

# GET an api.github.com path. Body on stdout; non-zero means not-found or
# unreachable. Every miss is logged with its status, so a rate-limited or
# offline run is legible as that rather than as a version having vanished.
api() {
    local path="$1"

    if [ -n "${PIN_API_FIXTURES:-}" ]; then
        local fixture="$PIN_API_FIXTURES/${path//\//__}.json"
        if [ ! -f "$fixture" ]; then
            printf 'api %s -> no fixture\n' "$path" >&2
            return 1
        fi
        cat "$fixture"
        return 0
    fi

    local auth=() body code
    if [ -n "${GITHUB_TOKEN:-}" ]; then
        auth=(-H "Authorization: Bearer $GITHUB_TOKEN")
    fi
    body="$(mktemp)"
    code=$(curl -sS -m 30 -o "$body" -w '%{http_code}' \
        -H 'Accept: application/vnd.github+json' \
        -H 'X-GitHub-Api-Version: 2022-11-28' \
        ${auth[@]+"${auth[@]}"} \
        "https://api.github.com/$path")
    if [ "$code" = "200" ]; then
        cat "$body"
        rm -f "$body"
        return 0
    fi
    printf 'api %s -> HTTP %s\n' "$path" "$code" >&2
    rm -f "$body"
    return 1
}

# Resolve a ref name in an upstream repo. Prints "<kind> <sha>":
#   tag <commit>    the commit the tag points at, annotated tags dereferenced
#   branch <head>   a moving ref; membership is checked by on_branch below
resolve_ref() {
    local repo="$1" ref="$2" body type sha

    if body=$(api "repos/$repo/git/ref/tags/$ref"); then
        type=$(printf '%s' "$body" | jq -r '.object.type')
        sha=$(printf '%s' "$body" | jq -r '.object.sha')
        if [ "$type" = "tag" ]; then
            body=$(api "repos/$repo/git/tags/$sha") || return 1
            sha=$(printf '%s' "$body" | jq -r '.object.sha')
        fi
        printf 'tag %s\n' "$sha"
        return 0
    fi

    if body=$(api "repos/$repo/git/ref/heads/$ref"); then
        printf 'branch %s\n' "$(printf '%s' "$body" | jq -r '.object.sha')"
        return 0
    fi

    return 1
}

# Is <sha> on <branch> upstream? An action published only from a branch (no
# release tags) can't be checked against a fixed commit — the branch head
# moves, which is why the SHA is pinned in the first place — but "this commit
# is on that branch" is the same statement the tag check makes, and holds.
# compare/<branch>...<sha> reports the pin as `identical` (it is the head) or
# `behind` (an ancestor of it).
on_branch() {
    local repo="$1" branch="$2" sha="$3" body status
    body=$(api "repos/$repo/compare/$branch...$sha") || return 1
    status=$(printf '%s' "$body" | jq -r '.status')
    [ "$status" = "identical" ] || [ "$status" = "behind" ]
}

declare -A resolved=()   # repo@version -> "<kind> <sha>", or "none -"
declare -A pinned_sha=() # repo -> the first SHA seen for it
declare -A pinned_at=()  # repo -> where that first SHA was seen

shopt -s nullglob
workflows=("$WORKFLOW_DIR"/*.yml "$WORKFLOW_DIR"/*.yaml)
if [ ${#workflows[@]} -eq 0 ]; then
    printf 'FAIL no workflow files under %s\n' "$WORKFLOW_DIR" >&2
    exit 1
fi

for file in "${workflows[@]}"; do
    lineno=0
    while IFS= read -r line || [ -n "$line" ]; do
        lineno=$((lineno + 1))

        # `- uses: <ref>` or `uses: <ref>`, with an optional trailing comment.
        # Anchored so a prose line mentioning uses: is not mistaken for one.
        [[ "$line" =~ ^[[:space:]]*(-[[:space:]]+)?uses:[[:space:]]*([^[:space:]#]+)[[:space:]]*(#[[:space:]]*(.*))?$ ]] || continue

        ref_spec="${BASH_REMATCH[2]}"
        comment="${BASH_REMATCH[4]:-}"
        where="$file:$lineno"

        # Local composite actions and container images are not upstream pins.
        case "$ref_spec" in
            ./* | docker://*) continue ;;
        esac

        pins=$((pins + 1))

        if [[ "$ref_spec" != *@* ]]; then
            fail "$where $ref_spec — no ref at all; pin it to a full commit SHA"
            continue
        fi
        action="${ref_spec%%@*}"
        pin="${ref_spec##*@}"
        repo="$(printf '%s' "$action" | cut -d/ -f1,2)"

        if ! [[ "$pin" =~ ^[0-9a-f]{40}$ ]]; then
            fail "$where $ref_spec — not a full 40-hex commit SHA; a tag or branch reference is what pinning prevents"
            continue
        fi

        # The claimed version is the first word of the comment, so both
        # `# v7.0.1` and `# stable branch, pinned` name a ref.
        claimed="${comment%%[[:space:]]*}"
        claimed="${claimed%[.,;:]}"
        if [ -z "$claimed" ]; then
            fail "$where $ref_spec — no version named in its comment; an unverifiable pin is not a verified one"
            continue
        fi

        # One action pinned at two SHAs: both individually valid, one job
        # silently older than the repository believes it standardized on.
        if [ -n "${pinned_sha[$repo]+set}" ]; then
            if [ "${pinned_sha[$repo]}" != "$pin" ]; then
                fail "$repo is pinned at two different commits: ${pinned_at[$repo]} and $where ($pin # $claimed)"
            fi
        else
            pinned_sha["$repo"]="$pin"
            pinned_at["$repo"]="$where ($pin # $claimed)"
        fi

        key="$repo@$claimed"
        if [ -z "${resolved[$key]+set}" ]; then
            if out=$(resolve_ref "$repo" "$claimed"); then
                resolved["$key"]="$out"
            else
                resolved["$key"]="none -"
            fi
        fi
        kind="${resolved[$key]%% *}"
        target="${resolved[$key]#* }"

        case "$kind" in
            none)
                fail "$where $ref_spec — comment claims $claimed, which does not resolve to a tag or branch in $repo"
                ;;
            tag)
                if [ "$target" != "$pin" ]; then
                    fail "$where $ref_spec — claims $claimed, but $claimed is $target upstream"
                fi
                ;;
            branch)
                if ! on_branch "$repo" "$claimed" "$pin"; then
                    fail "$where $ref_spec — claims branch $claimed, but that commit is not on $repo's $claimed branch"
                fi
                ;;
        esac
    done <"$file"
done

if [ "$failures" -ne 0 ]; then
    printf '%d of %d action pins failed verification.\n' "$failures" "$pins"
    exit 1
fi

printf 'Verified %d action pins across %d workflow files.\n' "$pins" "${#workflows[@]}"
