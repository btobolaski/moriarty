# Getting Started with rules

After acquiring the moriarty binary, the first thing you should do is set your initial rules. Writing the output of
`moriarty rules starter` to `~/.config/moriarty/tool_rules.toml` will do just that. After you have your initial config,
you also need to configure claude code to utilize the rules. Adding this section to `~/.claude/settings.json` will
enable hooks for bash commands (you can swap `"matcher": "Bash"` with `"matcher": "*"` to also enable tool rules):

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "moriarty hooks exec"
          }
        ]
      }
    ]
  }
}
```

If you'd like to utilize this with pi, you'll either need to modify one of the many pi-permission-system forks yourself
or utilize [my fork][1] and then add a similar hook setup in the
`~/.pi/agent/extensions/pi-permission-system/config.json` configuration:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": ".*",
        "hooks": [
          {
            "type": "command",
            "command": "moriarty hooks exec"
          }
        ]
      }
    ]
  }
}
```

[1]: https://github.com/btobolaski/pi-packages/tree/claude-hooks-v26.3.1

Once you have all of that in place it will be working but, it will not do very much. It will start collecting logs of
all attempted tool calls. From here, you have two options: the proactive and the reactive.

## Proactive

As you get prompted for approval to run tools, keep a list of them. Then ask an agent to read the documentation on the
rules and update them to allow those tools to be executed. You'll need to utilize this method for tool rules as rules
suggest doesn't currently produce suggestions for tool rules.

## Reactive

Just let the agent go about its business and moriarty will be collecting logs of the tool calls that the agents attempt
as well as the results. After you have run for a while, you can run `moriarty rules suggest` for a list of rules to add.
These will all generate as Ask but, you should modify them to Approve or Deny depending on what you want. rules suggest
does try to do some fuzzy matching but, it is not particularly good so, again, you'll probably want to have an agent
look at the suggestions and then write the rules.

## Interaction with Claude Code auto mode

If a given tool call is not fully matched by existing rules, moriarty will return an empty hook result. This means that
the agent can make its own decision. This allows for moriarty to be a pre-filter for Claude Code's auto mode. If you
never want to rely on auto mode making a decision, you'll need to add catch-all rules at the end of of `tool_rules.toml`
that set everything to ask.

## General guidance

I strongly recommend having a test suite for the rules, this way you can build confidence that the rules do not allow
harmful commands to be auto-approved and they also function as a way to prevent regressions. I use [bats][2] for this.
It works reasonably well but, it is a bit slow.

[2]: https://github.com/bats-core/bats-core
