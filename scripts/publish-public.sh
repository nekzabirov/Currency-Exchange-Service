#!/usr/bin/env bash
#
# Builds the public snapshot of this repository and syncs it into a local clone of the public repo.
#
#   scripts/publish-public.sh <export-dir>          # build, guard, sync into <export-dir>
#   scripts/publish-public.sh <export-dir> --push   # ... then commit + push, and sync the wiki
#
# Steps:
#   1. `git archive HEAD` into a temp dir (committed state only, never the working tree);
#   2. delete the paths listed in .public/exclude;
#   3. copy .public/overlay/ over the result;
#   4. run the forbidden-pattern guard (.public/forbidden-patterns), failing with file:line;
#   5. rsync the result into <export-dir>, keeping its .git.
#
# With --push: commit in <export-dir> as the public author and push to main, then replace the
# GitHub Wiki contents with docs/wiki/* and push it as well.

set -euo pipefail

PUBLIC_REPO=nekzabirov/ratekeeper

AUTHOR_NAME="Nekbakht Zabirov"
AUTHOR_EMAIL="17720956+nekzabirov@users.noreply.github.com"

# Allowed only in these paths (as a link), forbidden everywhere else.
PRODUCTION_LINK_PATTERN='1638\.cloud'
PRODUCTION_LINK_ALLOWED='^(README\.md|docs/wiki/.*)$'

usage() {
    echo "usage: $0 <export-dir> [--push]" >&2
    exit 2
}

EXPORT_DIR=""
PUSH=false

for arg in "$@"; do
    case "$arg" in
        --push) PUSH=true ;;
        -h | --help) usage ;;
        -*) echo "unknown option: $arg" >&2; usage ;;
        *)
            [[ -z "$EXPORT_DIR" ]] || usage
            EXPORT_DIR="$arg"
            ;;
    esac
done

[[ -n "$EXPORT_DIR" ]] || usage

ROOT="$(git rev-parse --show-toplevel)"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

STAGE="$WORK/stage"
META="$WORK/meta"
mkdir -p "$STAGE" "$META"

echo "==> Archiving HEAD ($(git -C "$ROOT" rev-parse --short HEAD))"
git -C "$ROOT" archive --format=tar HEAD | tar -x -C "$STAGE"

if [[ ! -d "$STAGE/.public" ]]; then
    echo "error: .public/ is missing from HEAD; nothing to publish from" >&2
    exit 1
fi
cp -R "$STAGE/.public" "$META/"

echo "==> Removing excluded paths"
# Works with the bash 3.2 that macOS ships: no globstar, so "any depth" goes through find.
remove_matches() {
    local pattern="$1"
    local path
    local matches=()
    if [[ "$pattern" == */* ]]; then
        shopt -s nullglob dotglob
        # shellcheck disable=SC2206 # globbing is the point here
        matches=( "$STAGE"/$pattern )
        shopt -u nullglob dotglob
    else
        while IFS= read -r path; do
            matches+=( "$path" )
        done < <(find "$STAGE" -name "$pattern" -prune)
    fi
    for path in ${matches[@]+"${matches[@]}"}; do
        if [[ -e "$path" || -L "$path" ]]; then
            echo "    - ${path#"$STAGE"/}"
            rm -rf "$path"
        fi
    done
}

while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line%%#*}"
    line="$(echo "$line" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
    [[ -n "$line" ]] || continue
    line="${line#/}"
    line="${line%/}"
    remove_matches "$line"
done < "$META/.public/exclude"

echo "==> Applying overlay"
if [[ -d "$META/.public/overlay" ]]; then
    (cd "$META/.public/overlay" && find . -type f | sed 's|^\./|    + |' | sort)
    cp -R "$META/.public/overlay/." "$STAGE/"
fi

echo "==> Running forbidden-pattern guard"
PATTERNS_FILE="$META/.public/forbidden-patterns" \
LINK_PATTERN="$PRODUCTION_LINK_PATTERN" \
LINK_ALLOWED="$PRODUCTION_LINK_ALLOWED" \
STAGE_DIR="$STAGE" \
perl -CSD -Mutf8 -e '
    use strict;
    use warnings;
    use File::Find;

    my @patterns;
    open my $pf, "<:encoding(UTF-8)", $ENV{PATTERNS_FILE} or die "cannot read patterns: $!\n";
    while (my $line = <$pf>) {
        chomp $line;
        $line =~ s/^\s+|\s+$//g;
        next if $line eq "" || $line =~ /^#/;
        push @patterns, [$line, qr/$line/i];
    }
    close $pf;

    my $link = qr/$ENV{LINK_PATTERN}/i;
    my $allowed = qr/$ENV{LINK_ALLOWED}/;
    my $root = $ENV{STAGE_DIR};
    my $hits = 0;

    my @files;
    find({ wanted => sub { push @files, $File::Find::name if -f $_ && !-l $_ }, no_chdir => 1 }, $root);

    for my $file (sort @files) {
        (my $rel = $file) =~ s/^\Q$root\E\///;
        my $check_name = sub {
            my ($text, $where) = @_;
            for my $p (@patterns) {
                if ($text =~ $p->[1]) { print "$where: matches /$p->[0]/\n"; $hits++; }
            }
        };
        $check_name->($rel, "$rel (path)");
        next if -B $file;
        open my $fh, "<:encoding(UTF-8)", $file or die "cannot read $rel: $!\n";
        my $n = 0;
        while (my $line = <$fh>) {
            $n++;
            for my $p (@patterns) {
                if ($line =~ $p->[1]) {
                    chomp(my $shown = $line);
                    $shown = substr($shown, 0, 160);
                    print "$rel:$n: /$p->[0]/: $shown\n";
                    $hits++;
                }
            }
            if ($line =~ $link && $rel !~ $allowed) {
                print "$rel:$n: /$ENV{LINK_PATTERN}/ outside README.md and docs/wiki/\n";
                $hits++;
            }
        }
        close $fh;
    }

    if ($hits) {
        print STDERR "guard: $hits forbidden match(es); nothing was exported\n";
        exit 1;
    }
    print "    guard passed (" . scalar(@files) . " files, " . scalar(@patterns) . " patterns)\n";
'

echo "==> Syncing into $EXPORT_DIR"
mkdir -p "$EXPORT_DIR"
rsync -a --delete --exclude '/.git' "$STAGE/" "$EXPORT_DIR/"

if [[ "$PUSH" != true ]]; then
    echo "==> Done (no push). Review the result in $EXPORT_DIR"
    exit 0
fi

commit_as_author() {
    local dir="$1" message="$2"
    git -C "$dir" add -A
    if git -C "$dir" diff --cached --quiet; then
        echo "    no changes in $dir"
        return 1
    fi
    git -C "$dir" \
        -c user.name="$AUTHOR_NAME" -c user.email="$AUTHOR_EMAIL" \
        commit --quiet --author="$AUTHOR_NAME <$AUTHOR_EMAIL>" -m "$message"
}

MESSAGE="Sync with upstream ($(date -u +%Y-%m-%d))"

echo "==> Committing and pushing $PUBLIC_REPO"
if [[ ! -d "$EXPORT_DIR/.git" ]]; then
    echo "error: $EXPORT_DIR is not a git repository (git init -b main / clone the public repo first)" >&2
    exit 1
fi
if ! git -C "$EXPORT_DIR" remote get-url origin >/dev/null 2>&1; then
    git -C "$EXPORT_DIR" remote add origin "git@github.com:${PUBLIC_REPO}.git"
fi
if commit_as_author "$EXPORT_DIR" "$MESSAGE"; then
    git -C "$EXPORT_DIR" push origin HEAD:main
fi

echo "==> Syncing the wiki"
WIKI="$WORK/wiki"
if ! git clone --quiet "git@github.com:${PUBLIC_REPO}.wiki.git" "$WIKI"; then
    echo "error: cannot clone the wiki; create its first page once in the GitHub UI, then rerun" >&2
    exit 1
fi
find "$WIKI" -mindepth 1 -maxdepth 1 ! -name .git -exec rm -rf {} +
cp -R "$STAGE/docs/wiki/." "$WIKI/"
if commit_as_author "$WIKI" "$MESSAGE"; then
    git -C "$WIKI" push origin HEAD
fi

echo "==> Done"
