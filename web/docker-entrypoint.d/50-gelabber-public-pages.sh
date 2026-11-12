#!/bin/sh
# Publishes the public pages (#522) and the feedback form (#542): fills the
# templates the build wrote
# (src/public/render.tsx) from the GELABBER_* values and the operator's texts
# in /etc/gelabber/legal, and writes public-pages.json, robots.txt and
# sitemap.xml. A page whose required values or texts are missing is left out,
# so no placeholder is ever shown. Never fails the container: the worst case
# is an instance without public pages, as before.

ME=$(basename "$0")
# The overrides are for the unit test (src/public/entrypoint.test.mjs).
TEMPLATES=${GELABBER_PUBLIC_TEMPLATES:-/usr/share/nginx/html/public-pages/templates}
OUT=${GELABBER_PUBLIC_OUT:-/usr/share/nginx/public-pages}
LEGAL=${GELABBER_LEGAL_DIR:-/etc/gelabber/legal}
# In a subfolder, so the storage map's `include /etc/nginx/gelabber/*.map;`
# does not pick it up.
MAP=${GELABBER_LANDING_MAP:-/etc/nginx/gelabber/public/landing.map}
SEE='(deploy/README.md, "Public pages")'
LEGAL_PAGES="imprint privacy terms"
# The feedback form and its four answer pages, per language.
FEEDBACK_DE="feedback feedback/gesendet feedback/danke feedback/abgelaufen feedback/fehler"
FEEDBACK_EN="feedback feedback/sent feedback/thanks feedback/expired feedback/error"

log() {
    [ -n "${NGINX_ENTRYPOINT_QUIET_LOGS:-}" ] || echo "$ME: $*"
}

# One line, without the spaces around it.
clean() {
    printf '%s' "$1" | tr '\r\n\t' '   ' | sed -e 's/^ *//' -e 's/ *$//'
}

# A value the templates read as {{KEY}}; empty drops the blocks of KEY.
set_value() {
    export "PUBLIC_VALUE_$1=$2"
}

other_lang() {
    if [ "$1" = de ]; then echo en; else echo de; fi
}

slug() {
    case "$1:$2" in
    de:imprint) echo impressum ;;
    de:privacy) echo datenschutz ;;
    de:terms) echo nutzungsbedingungen ;;
    *) echo "$2" ;;
    esac
}

# An operator text the page can show.
has_text() {
    [ -f "$1" ] && [ -r "$1" ] && [ -s "$1" ]
}

xml_escape() {
    printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' \
        -e 's/"/\&quot;/g' -e "s/'/\&#39;/g"
}

# Values are HTML-escaped and never read again, so a value cannot add a
# marker or a placeholder. Every awk takes ENVIRON as it is, without the
# escape processing of `-v`; nothing goes through gsub, whose replacement
# treats `&` and `\` specially.
# shellcheck disable=SC2016
FILL='
function escape(text,    out, c, i) {
    out = ""
    for (i = 1; i <= length(text); i++) {
        c = substr(text, i, 1)
        if (c == "&") c = "&amp;"
        else if (c == "<") c = "&lt;"
        else if (c == ">") c = "&gt;"
        else if (c == "\"") c = "&quot;"
        else if (c == "\047") c = "&#39;"
        out = out c
    }
    return out
}
function value(key) {
    return ENVIRON["PUBLIC_VALUE_" key]
}
function fill(line,    out, i, j, key, rest) {
    out = ""
    while ((i = index(line, "{{")) > 0) {
        rest = substr(line, i + 2)
        j = index(rest, "}}")
        key = j > 0 ? substr(rest, 1, j - 1) : ""
        if (key !~ /^[A-Z][A-Z0-9_]*$/) {
            out = out substr(line, 1, i + 1)
            line = rest
            continue
        }
        out = out substr(line, 1, i - 1) escape(value(key))
        line = substr(rest, j + 2)
    }
    return out line
}
/^[ \t]*<!--if:[A-Z0-9_]+-->[ \t]*$/ {
    key = $0
    sub(/^[ \t]*<!--if:/, "", key)
    sub(/-->[ \t]*$/, "", key)
    depth++
    if (!skip && value(key) == "") skip = depth
    next
}
/^[ \t]*<!--end:[A-Z0-9_]+-->[ \t]*$/ {
    if (depth == 0) { broken = 1; exit }
    if (skip == depth) skip = 0
    depth--
    next
}
skip { next }
/^[ \t]*<!--gelabber:fragment [a-z]+-->[ \t]*$/ {
    file = ENVIRON["PUBLIC_FRAGMENT"]
    if (file != "") {
        while ((status = (getline text < file)) > 0) print text
        close(file)
        if (status < 0) { broken = 1; exit }
    }
    next
}
{ print fill($0) }
END { if (broken || depth != 0) exit 1 }
'

# render LANG FILE [TEXT]: one page from its template, with the operator's
# TEXT at its fragment marker. FILE may sit in a folder (feedback/danke).
render() {
    mkdir -p "$(dirname "$OUT/$1/$2.html")" 2>/dev/null &&
        PUBLIC_FRAGMENT=${3:-} LC_ALL=C awk "$FILL" "$TEMPLATES/$1/$2.html" \
            >"$OUT/$1/$2.html" 2>/dev/null
}

# What this start publishes, all of it rewritten.
rm -rf "$OUT/de" "$OUT/en" "$OUT/public-pages.json" "$OUT/robots.txt" \
    "$OUT/sitemap.xml" "$MAP" 2>/dev/null
if ! mkdir -p "$OUT" 2>/dev/null; then
    log "warning: cannot write $OUT; no public pages are published $SEE"
    exit 0
fi

# A plain address for a mailto: link, with nothing like ?bcc= after it.
is_email() {
    printf '%s' "$1" | grep -Eq '^[^@ ?#/:]+@[^@ ?#/:]+$'
}

is_http_url() {
    case "$1" in
    [Hh][Tt][Tt][Pp]://?* | [Hh][Tt][Tt][Pp][Ss]://?*) return 0 ;;
    *) return 1 ;;
    esac
}

# checked NAME VALUE TEST WHAT: sets $checked to the cleaned VALUE if TEST
# accepts it, else to empty with a warning, so a link never gets an unusable
# target.
checked() {
    checked=$(clean "$2")
    if [ -n "$checked" ] && ! "$3" "$checked"; then
        log "warning: $1 is not $4; it is not used $SEE"
        checked=
    fi
}

# Values.
name=$(clean "${GELABBER_OPERATOR_NAME:-}")
street=$(clean "${GELABBER_OPERATOR_STREET:-}")
city=$(clean "${GELABBER_OPERATOR_CITY:-}")
checked GELABBER_OPERATOR_EMAIL "${GELABBER_OPERATOR_EMAIL:-}" \
    is_email "an email address"
email=$checked
checked GELABBER_CONTACT_EMAIL "${GELABBER_CONTACT_EMAIL:-}" \
    is_email "an email address"
contact=$checked
set_value GELABBER_OPERATOR_NAME "$name"
set_value GELABBER_OPERATOR_STREET "$street"
set_value GELABBER_OPERATOR_CITY "$city"
set_value GELABBER_OPERATOR_EMAIL "$email"
set_value GELABBER_OPERATOR_PHONE "$(clean "${GELABBER_OPERATOR_PHONE:-}")"
set_value GELABBER_CONTACT_EMAIL "$contact"

responsible=$(clean "${GELABBER_CONTENT_RESPONSIBLE:-}")
[ -n "$responsible" ] || responsible="$name, $street, $city"
set_value GELABBER_CONTENT_RESPONSIBLE "$responsible"
checked GELABBER_REPORT_EMAIL "${GELABBER_REPORT_EMAIL:-}" \
    is_email "an email address"
set_value GELABBER_REPORT_EMAIL "${checked:-$email}"

checked GELABBER_SOURCE_URL "${GELABBER_SOURCE_URL:-}" \
    is_http_url "an http(s):// URL"
set_value GELABBER_SOURCE_URL \
    "${checked:-https://github.com/Till-und-Scholler-AI-slop/Gelabber}"
checked GELABBER_OPERATOR_GUIDE_URL "${GELABBER_OPERATOR_GUIDE_URL:-}" \
    is_http_url "an http(s):// URL"
set_value GELABBER_OPERATOR_GUIDE_URL \
    "${checked:-https://github.com/Till-und-Scholler-AI-slop/Gelabber/blob/main/deploy/README.md}"

checked APP_PUBLIC_URL "${APP_PUBLIC_URL:-}" \
    is_http_url "an http(s):// URL"
public_url=$checked
while [ "${public_url%/}" != "$public_url" ]; do public_url=${public_url%/}; done
set_value APP_PUBLIC_URL "$public_url"

# "Request an invite": an https:// URL or an email address, else the contact
# address; with neither the buttons are left out.
invite=$(clean "${GELABBER_INVITE_REQUEST:-}")
invite_url=
invite_email=
case "$invite" in
"") ;;
[Hh][Tt][Tt][Pp][Ss]://?*) invite_url=$invite ;;
*)
    if is_email "$invite"; then
        invite_email=$invite
    else
        log "warning: GELABBER_INVITE_REQUEST is neither an https:// URL nor an email address $SEE"
    fi
    ;;
esac
[ -n "$invite_url$invite_email" ] || invite_email=$contact
set_value INVITE_URL "$invite_url"
set_value INVITE_EMAIL "$invite_email"

# Which pages are published. A page needs its templates in both languages.
templates_ok() {
    [ -r "$TEMPLATES/de/$(slug de "$1").html" ] &&
        [ -r "$TEMPLATES/en/$(slug en "$1").html" ]
}

landing=false
if [ "$(clean "${GELABBER_LANDING:-}" | tr 'A-Z' 'a-z')" = true ]; then
    if templates_ok index; then
        landing=true
    else
        log "warning: no landing page template in $TEMPLATES; the landing page is not published $SEE"
    fi
else
    log "the landing page is off: GELABBER_LANDING is not true $SEE"
fi

feedback_pages() {
    if [ "$1" = de ]; then echo "$FEEDBACK_DE"; else echo "$FEEDBACK_EN"; fi
}

# The feedback form needs the landing page as its front door, and an API that
# sends account mail and names operators; Compose derives the last two from
# SMTP_HOST and API_OPERATOR_EMAILS.
feedback=false
reason=
[ "$landing" = true ] || reason="the landing page is off"
[ -n "$(clean "${GELABBER_ACCOUNT_MAIL:-}")" ] ||
    reason="${reason:+$reason, }GELABBER_ACCOUNT_MAIL is empty"
[ -n "$(clean "${GELABBER_FEEDBACK_OPERATORS:-}")" ] ||
    reason="${reason:+$reason, }GELABBER_FEEDBACK_OPERATORS is empty"
missing_template=
for lang in de en; do
    for file in $(feedback_pages "$lang"); do
        [ -r "$TEMPLATES/$lang/$file.html" ] ||
            missing_template=1
    done
done
[ -z "$missing_template" ] ||
    reason="${reason:+$reason, }its templates are missing in $TEMPLATES"
if [ -n "$reason" ]; then
    log "the feedback form is not published: $reason $SEE"
else
    feedback=true
fi

published=
for page in $LEGAL_PAGES; do
    reason=
    if [ "$page" = imprint ]; then
        missing=
        [ -n "$name" ] || missing="$missing GELABBER_OPERATOR_NAME"
        [ -n "$street" ] || missing="$missing GELABBER_OPERATOR_STREET"
        [ -n "$city" ] || missing="$missing GELABBER_OPERATOR_CITY"
        [ -n "$email" ] || missing="$missing GELABBER_OPERATOR_EMAIL"
        [ -z "$missing" ] || reason="missing${missing}"
    elif ! has_text "$LEGAL/$page.de.html" && ! has_text "$LEGAL/$page.en.html"; then
        reason="no $page.de.html or $page.en.html in $LEGAL"
    fi
    if [ -z "$reason" ] && ! templates_ok "$page"; then
        reason="its templates are missing in $TEMPLATES"
    fi
    if [ -n "$reason" ]; then
        log "the $page page is not published: $reason $SEE"
    else
        published="$published $page"
    fi
done

# The legal pages and the feedback form. The operator's text in one language
# only is shown under both, with a notice in the other. A page that cannot be
# written is dropped and the others rendered again, so none links to it.
# PAGE_FEEDBACK is set first: the legal pages and the landing page link it.
# The landing page comes last, with the final flags.
while :; do
    for page in $LEGAL_PAGES; do
        case " $published " in
        *" $page "*) on=1 ;;
        *) on= ;;
        esac
        set_value "PAGE_$(echo "$page" | tr 'a-z' 'A-Z')" "$on"
    done
    if [ "$feedback" = true ]; then
        set_value PAGE_FEEDBACK 1
    else
        set_value PAGE_FEEDBACK ""
    fi
    dropped=
    for page in $published; do
        ok=1
        for lang in de en; do
            other=$(other_lang "$lang")
            text=
            set_value FRAGMENT_LANG "$lang"
            set_value ONLY_OTHER_LANGUAGE ""
            if has_text "$LEGAL/$page.$lang.html"; then
                text="$LEGAL/$page.$lang.html"
            elif has_text "$LEGAL/$page.$other.html"; then
                text="$LEGAL/$page.$other.html"
                set_value FRAGMENT_LANG "$other"
                set_value ONLY_OTHER_LANGUAGE 1
            fi
            render "$lang" "$(slug "$lang" "$page")" "$text" || ok=
        done
        if [ -z "$ok" ]; then
            rm -f "$OUT/de/$(slug de "$page").html" "$OUT/en/$(slug en "$page").html"
            log "warning: cannot write the $page page to $OUT; it is not published $SEE"
            published=$(printf '%s' " $published " | sed "s/ $page / /")
            dropped=1
        fi
    done
    # The form links the privacy page, so it is written after the legal
    # pages, with their final flags.
    if [ "$feedback" = true ]; then
        ok=1
        for lang in de en; do
            for file in $(feedback_pages "$lang"); do
                render "$lang" "$file" || ok=
            done
        done
        if [ -z "$ok" ]; then
            rm -rf "$OUT/de/feedback.html" "$OUT/de/feedback" \
                "$OUT/en/feedback.html" "$OUT/en/feedback"
            log "warning: cannot write the feedback form to $OUT; it is not published $SEE"
            feedback=false
            dropped=1
        fi
    fi
    # The landing page links both. Without it the form is dropped too: the
    # landing page is its front door.
    if [ "$landing" = true ] && ! { render de index && render en index; }; then
        rm -f "$OUT/de/index.html" "$OUT/en/index.html"
        landing=false
        log "warning: cannot write the landing page to $OUT; it is not published $SEE"
        if [ "$feedback" = true ]; then
            rm -rf "$OUT/de/feedback.html" "$OUT/de/feedback" \
                "$OUT/en/feedback.html" "$OUT/en/feedback"
            log "the feedback form is not published: the landing page is off $SEE"
            feedback=false
            dropped=1
        fi
    fi
    [ -n "$dropped" ] || break
done

if [ "$landing" = true ]; then
    case " $published " in
    *" imprint "*) ;;
    *) log "warning: the landing page is published without an Impressum $SEE" ;;
    esac
fi

# /public-pages.json, read by the sign-in pages and the desktop app.
{
    printf '{\n  "version": 1,\n  "landing": %s,\n  "legal": {' "$landing"
    separator=
    for page in $published; do
        printf '%s\n    "%s": { "de": "/de/%s", "en": "/en/%s" }' "$separator" \
            "$page" "$(slug de "$page")" "$(slug en "$page")"
        separator=,
    done
    [ -z "$published" ] || printf '\n  '
    printf '}\n}\n'
} >"$OUT/public-pages.json" 2>/dev/null ||
    log "warning: cannot write $OUT/public-pages.json $SEE"

# robots.txt, and sitemap.xml with the absolute URLs it needs.
{
    echo "User-agent: *"
    if [ "$landing" = true ]; then
        echo 'Allow: /$'
        echo "Allow: /de/"
        echo "Allow: /en/"
        for page in $published; do
            echo "Allow: /de/$(slug de "$page")"
            echo "Allow: /en/$(slug en "$page")"
        done
        for path in /login /register /forgot-password /reset-password \
            /verify-email /invite/ /s/ /d /settings /api/ /media/ /ws \
            /de/feedback /en/feedback; do
            echo "Disallow: $path"
        done
        [ -z "$public_url" ] || echo "Sitemap: $public_url/sitemap.xml"
    else
        echo "Disallow: /"
    fi
} >"$OUT/robots.txt" 2>/dev/null || log "warning: cannot write $OUT/robots.txt $SEE"

if [ "$landing" = true ] && [ -n "$public_url" ]; then
    base=$(xml_escape "$public_url")
    {
        echo '<?xml version="1.0" encoding="UTF-8"?>'
        echo '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">'
        for path in /de/ /en/; do
            echo "  <url><loc>$base$path</loc></url>"
        done
        for page in $published; do
            echo "  <url><loc>$base/de/$(slug de "$page")</loc></url>"
            echo "  <url><loc>$base/en/$(slug en "$page")</loc></url>"
        done
        echo '</urlset>'
    } >"$OUT/sitemap.xml" 2>/dev/null || log "warning: cannot write $OUT/sitemap.xml $SEE"
fi

# Last, so nginx serves the landing page at / only once it exists.
if [ "$landing" = true ]; then
    if mkdir -p "$(dirname "$MAP")" 2>/dev/null &&
        printf '"" 1;\n' >"$MAP" 2>/dev/null; then
        log "the landing page is published at / for signed-out visitors"
    else
        log "warning: cannot write $MAP; / stays the app $SEE"
    fi
fi
[ "$feedback" = false ] || published="$published feedback"
log "published:${published:- no legal pages}"
exit 0
