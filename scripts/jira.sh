#!/usr/bin/env bash
# Jira CLI wrapper — replaces broken mcp-atlassian MCP server
# Uses the new /rest/api/3/search/jql endpoint
#
# Usage: ./scripts/jira.sh <command> [args...]
#
# Commands:
#   projects                          List all projects
#   issues <PROJECT> [maxResults]     List issues for a project
#   issue <KEY>                       Get issue details
#   search <JQL> [maxResults]         Search with JQL
#   sprint-issues <SPRINT_ID>         List issues in a sprint
#   sprints <BOARD_ID> [state]        List sprints (state: active,future,closed)
#   boards [projectKey]               List boards
#   transition <KEY> <TRANSITION_ID>  Transition an issue
#   transitions <KEY>                 List available transitions
#   comment <KEY> <TEXT>              Add a comment to an issue
#   assign <KEY> <ACCOUNT_ID>        Assign an issue
#   status <KEY> <STATUS>            Shortcut: transition by status name

set -euo pipefail

JIRA_URL="${JIRA_URL:-https://steffx115.atlassian.net}"
JIRA_USER="${JIRA_USER:-steff@gornik.at}"
JIRA_TOKEN="${JIRA_API_TOKEN:-}"

if [[ -z "$JIRA_TOKEN" ]]; then
  echo "Error: JIRA_API_TOKEN env var not set" >&2
  exit 1
fi

AUTH="$JIRA_USER:$JIRA_TOKEN"
API="$JIRA_URL/rest/api/3"
AGILE="$JIRA_URL/rest/agile/1.0"

_curl() {
  curl -s -u "$AUTH" -H "Content-Type: application/json" "$@"
}

_jq() {
  node -e "
    const d=[];
    process.stdin.on('data',c=>d.push(c));
    process.stdin.on('end',()=>{
      try {
        const r=JSON.parse(d.join(''));
        const fn=new Function('r',\`$1\`);
        fn(r);
      } catch(e) {
        console.error('Parse error:',e.message);
        console.error('Raw:', d.join('').slice(0,500));
        process.exit(1);
      }
    });
  "
}

cmd_projects() {
  _curl "$API/project" | _jq '
    r.forEach(p => console.log(p.key + " | " + p.name));
  '
}

cmd_issues() {
  local project="${1:?Usage: jira.sh issues <PROJECT> [maxResults]}"
  local max="${2:-20}"
  local jql="project=$project ORDER BY created DESC"
  _curl "$API/search/jql?jql=$(printf %s "$jql" | node -e "process.stdin.on('data',d=>process.stdout.write(encodeURIComponent(d.toString())))")&maxResults=$max&fields=summary,status,priority,issuetype,assignee" \
    | _jq '
      (r.issues||[]).forEach(i => {
        const f = i.fields;
        console.log([
          i.key,
          f.issuetype.name,
          f.status.name,
          (f.priority||{}).name||"-",
          f.summary
        ].join(" | "));
      });
    '
}

cmd_issue() {
  local key="${1:?Usage: jira.sh issue <KEY>}"
  _curl "$API/issue/$key?fields=summary,description,status,priority,issuetype,assignee,labels" \
    | _jq '
      const f = r.fields;
      console.log("Key:      " + r.key);
      console.log("Summary:  " + f.summary);
      console.log("Type:     " + f.issuetype.name);
      console.log("Status:   " + f.status.name);
      console.log("Priority: " + (f.priority||{}).name||"-");
      console.log("Assignee: " + (f.assignee?f.assignee.displayName:"Unassigned"));
      console.log("Labels:   " + (f.labels||[]).join(", "));
      if (f.description && f.description.content) {
        const text = [];
        const walk = n => { if(n.text) text.push(n.text); if(n.content) n.content.forEach(walk); };
        f.description.content.forEach(walk);
        console.log("\nDescription:\n" + text.join(" "));
      }
    '
}

cmd_search() {
  local jql="${1:?Usage: jira.sh search <JQL> [maxResults]}"
  local max="${2:-20}"
  _curl "$API/search/jql?jql=$(printf %s "$jql" | node -e "process.stdin.on('data',d=>process.stdout.write(encodeURIComponent(d.toString())))")&maxResults=$max&fields=summary,status,priority,issuetype,assignee" \
    | _jq '
      (r.issues||[]).forEach(i => {
        const f = i.fields;
        console.log([
          i.key,
          f.issuetype.name,
          f.status.name,
          (f.priority||{}).name||"-",
          f.summary
        ].join(" | "));
      });
    '
}

cmd_boards() {
  local project="${1:-}"
  local url="$AGILE/board"
  [[ -n "$project" ]] && url="$url?projectKeyOrId=$project"
  _curl "$url" | _jq '
    r.values.forEach(b => console.log(b.id + " | " + b.name + " | " + b.type));
  '
}

cmd_sprints() {
  local board="${1:?Usage: jira.sh sprints <BOARD_ID> [state]}"
  local state="${2:-active,future}"
  _curl "$AGILE/board/$board/sprint?state=$state" | _jq '
    r.values.forEach(s => console.log([
      s.id, s.name, s.state, s.startDate||"-", s.endDate||"-"
    ].join(" | ")));
  '
}

cmd_sprint_issues() {
  local sprint="${1:?Usage: jira.sh sprint-issues <SPRINT_ID>}"
  local max="${2:-50}"
  _curl "$AGILE/sprint/$sprint/issue?maxResults=$max&fields=summary,status,priority,issuetype,assignee" \
    | _jq '
      r.issues.forEach(i => {
        const f = i.fields;
        console.log([
          i.key,
          f.issuetype.name,
          f.status.name,
          (f.priority||{}).name||"-",
          f.summary
        ].join(" | "));
      });
    '
}

cmd_transitions() {
  local key="${1:?Usage: jira.sh transitions <KEY>}"
  _curl "$API/issue/$key/transitions" | _jq '
    r.transitions.forEach(t => console.log(t.id + " | " + t.name));
  '
}

cmd_transition() {
  local key="${1:?Usage: jira.sh transition <KEY> <TRANSITION_ID>}"
  local tid="${2:?Usage: jira.sh transition <KEY> <TRANSITION_ID>}"
  _curl -X POST "$API/issue/$key/transitions" \
    -d "{\"transition\":{\"id\":\"$tid\"}}" \
    && echo "Transitioned $key"
}

cmd_status() {
  local key="${1:?Usage: jira.sh status <KEY> <STATUS_NAME>}"
  local target="${2:?Usage: jira.sh status <KEY> <STATUS_NAME>}"
  local target_lower
  target_lower=$(echo "$target" | tr '[:upper:]' '[:lower:]')

  local result
  result=$(_curl "$API/issue/$key/transitions" | _jq "
    const t = r.transitions.find(t => t.name.toLowerCase() === '$target_lower');
    if (t) { console.log(t.id); } else {
      console.error('Available transitions: ' + r.transitions.map(t=>t.name).join(', '));
      process.exit(1);
    }
  ")

  if [[ -n "$result" ]]; then
    _curl -X POST "$API/issue/$key/transitions" \
      -d "{\"transition\":{\"id\":\"$result\"}}" \
      && echo "Transitioned $key to $target"
  fi
}

cmd_comment() {
  local key="${1:?Usage: jira.sh comment <KEY> <TEXT>}"
  local text="${2:?Usage: jira.sh comment <KEY> <TEXT>}"
  _curl -X POST "$API/issue/$key/comment" \
    -d "{\"body\":{\"version\":1,\"type\":\"doc\",\"content\":[{\"type\":\"paragraph\",\"content\":[{\"type\":\"text\",\"text\":\"$text\"}]}]}}" \
    | _jq 'console.log("Comment added: " + r.id)'
}

cmd_assign() {
  local key="${1:?Usage: jira.sh assign <KEY> <ACCOUNT_ID>}"
  local account="${2:?Usage: jira.sh assign <KEY> <ACCOUNT_ID>}"
  _curl -X PUT "$API/issue/$key/assignee" \
    -d "{\"accountId\":\"$account\"}" \
    && echo "Assigned $key to $account"
}

# Dispatch
case "${1:-help}" in
  projects)       shift; cmd_projects "$@" ;;
  issues)         shift; cmd_issues "$@" ;;
  issue)          shift; cmd_issue "$@" ;;
  search)         shift; cmd_search "$@" ;;
  boards)         shift; cmd_boards "$@" ;;
  sprints)        shift; cmd_sprints "$@" ;;
  sprint-issues)  shift; cmd_sprint_issues "$@" ;;
  transitions)    shift; cmd_transitions "$@" ;;
  transition)     shift; cmd_transition "$@" ;;
  status)         shift; cmd_status "$@" ;;
  comment)        shift; cmd_comment "$@" ;;
  assign)         shift; cmd_assign "$@" ;;
  help|--help|-h)
    head -14 "$0" | tail -13
    ;;
  *)
    echo "Unknown command: $1" >&2
    echo "Run '$0 help' for usage" >&2
    exit 1
    ;;
esac
