#!/usr/bin/env python3
"""
Splitrail Token Usage Analyzer
Extracts and aggregates token usage across all supported AI coding tools.

Usage:
    python3 scripts/analyze_tokens.py
    python3 scripts/analyze_tokens.py --json
    python3 scripts/analyze_tokens.py --csv
    python3 scripts/analyze_tokens.py --tool claude_code
"""

import json
import re
import os
import sys
import argparse
from glob import glob
from datetime import datetime
from collections import defaultdict
from pathlib import Path


class TokenAnalyzer:
    TOOLS = ['claude_code', 'codex_cli', 'gemini_cli', 'opencode', 'kilo_code', 'qwen_code', 'warp', 'cline']

    def __init__(self):
        self.home = str(Path.home())
        self.daily_stats = defaultdict(lambda: defaultdict(lambda: {
            'input': 0, 'output': 0, 'total': 0
        }))

    def analyze_claude_code(self):
        """Extract Claude Code token usage from JSONL files."""
        pattern = f"{self.home}/.claude/projects/*/*.jsonl"

        for filepath in glob(pattern):
            try:
                with open(filepath, 'r') as f:
                    for line in f:
                        try:
                            data = json.loads(line)
                            if data.get('type') == 'assistant':
                                usage = data.get('message', {}).get('usage', {})
                                if usage:
                                    ts = data.get('timestamp', '')
                                    date = ts[:10] if ts else None
                                    if date:
                                        self.daily_stats[date]['claude_code']['input'] += usage.get('input_tokens', 0)
                                        self.daily_stats[date]['claude_code']['output'] += usage.get('output_tokens', 0)
                        except json.JSONDecodeError:
                            continue
            except Exception as e:
                if os.getenv('DEBUG'):
                    print(f"Error processing {filepath}: {e}", file=sys.stderr)

    def analyze_codex_cli(self):
        """Extract Codex CLI token usage, handling cumulative totals correctly."""
        pattern = f"{self.home}/.codex/sessions/*/*/*/*.jsonl"

        for filepath in glob(pattern):
            # Extract date from path: sessions/YYYY/MM/DD/file.jsonl
            parts = filepath.split('/')
            try:
                date = f"{parts[-4]}-{parts[-3]}-{parts[-2]}"
            except IndexError:
                continue

            max_input = 0
            max_output = 0

            try:
                with open(filepath, 'r') as f:
                    for line in f:
                        # Extract cumulative totals (take MAX, not sum)
                        match = re.search(r'"total_token_usage":\{[^}]*"input_tokens":(\d+)[^}]*"output_tokens":(\d+)', line)
                        if match:
                            inp = int(match.group(1))
                            out = int(match.group(2))
                            max_input = max(max_input, inp)
                            max_output = max(max_output, out)
            except Exception as e:
                if os.getenv('DEBUG'):
                    print(f"Error processing {filepath}: {e}", file=sys.stderr)
                continue

            self.daily_stats[date]['codex_cli']['input'] += max_input
            self.daily_stats[date]['codex_cli']['output'] += max_output

    def analyze_gemini_cli(self):
        """Extract Gemini CLI token usage from JSON chat files."""
        pattern = f"{self.home}/.gemini/tmp/*/chats/*.json"

        for filepath in glob(pattern):
            try:
                with open(filepath, 'r') as f:
                    data = json.load(f)

                # Get date from file modification time
                mtime = os.path.getmtime(filepath)
                date = datetime.fromtimestamp(mtime).strftime('%Y-%m-%d')

                for msg in data.get('messages', []):
                    # Gemini CLI uses 'tokens' field with input/output/cached/thoughts/tool
                    tokens = msg.get('tokens', {})
                    if tokens:
                        self.daily_stats[date]['gemini_cli']['input'] += tokens.get('input', 0)
                        self.daily_stats[date]['gemini_cli']['output'] += tokens.get('output', 0)
            except Exception as e:
                if os.getenv('DEBUG'):
                    print(f"Error processing {filepath}: {e}", file=sys.stderr)

    def analyze_opencode(self):
        """Extract OpenCode token usage from message JSON files."""
        pattern = f"{self.home}/.local/share/opencode/storage/message/*/msg_*.json"

        for filepath in glob(pattern):
            try:
                with open(filepath, 'r') as f:
                    data = json.load(f)

                # OpenCode uses 'tokens' with input/output/reasoning
                tokens = data.get('tokens', {})
                if tokens:
                    mtime = os.path.getmtime(filepath)
                    date = datetime.fromtimestamp(mtime).strftime('%Y-%m-%d')

                    self.daily_stats[date]['opencode']['input'] += tokens.get('input', 0)
                    self.daily_stats[date]['opencode']['output'] += tokens.get('output', 0)
            except Exception as e:
                if os.getenv('DEBUG'):
                    print(f"Error processing {filepath}: {e}", file=sys.stderr)

    def analyze_kilo_code(self):
        """Extract Kilo Code token usage from embedded JSON in text fields."""
        patterns = [
            f"{self.home}/.kilocode/cli/global/tasks/*/ui_messages.json",
            f"{self.home}/.vscode/data/User/globalStorage/kilocode.kilo-code/tasks/*/ui_messages.json",
            f"{self.home}/.cursor/data/User/globalStorage/kilocode.kilo-code/tasks/*/ui_messages.json",
            f"{self.home}/.windsurf/data/User/globalStorage/kilocode.kilo-code/tasks/*/ui_messages.json",
        ]

        for pattern in patterns:
            for filepath in glob(pattern):
                try:
                    with open(filepath, 'r') as f:
                        data = json.load(f)

                    mtime = os.path.getmtime(filepath)
                    date = datetime.fromtimestamp(mtime).strftime('%Y-%m-%d')

                    # Kilo Code data is a flat list of messages, not {"messages": [...]}
                    messages = data if isinstance(data, list) else data.get('messages', [])

                    for msg in messages:
                        text = msg.get('text', '')

                        # Extract embedded JSON tokens (e.g., {"tokensIn":10968,"tokensOut":73,...})
                        in_match = re.search(r'"tokensIn":(\d+)', text)
                        out_match = re.search(r'"tokensOut":(\d+)', text)

                        if in_match:
                            self.daily_stats[date]['kilo_code']['input'] += int(in_match.group(1))
                        if out_match:
                            self.daily_stats[date]['kilo_code']['output'] += int(out_match.group(1))
                except Exception as e:
                    if os.getenv('DEBUG'):
                        print(f"Error processing {filepath}: {e}", file=sys.stderr)

    def analyze_qwen_code(self):
        """Extract Qwen Code token usage from chat and stats files."""
        patterns = [
            f"{self.home}/.qwen/tmp/*/chats/*.json",
            f"{self.home}/.qwen/auto_stats/*-stats.jsonl",  # Only stats files have tokens
        ]

        for pattern in patterns:
            for filepath in glob(pattern):
                try:
                    mtime = os.path.getmtime(filepath)
                    date = datetime.fromtimestamp(mtime).strftime('%Y-%m-%d')

                    if filepath.endswith('.jsonl'):
                        # Track max tokens per model to handle cumulative stats
                        model_max = {}
                        with open(filepath, 'r') as f:
                            for line in f:
                                try:
                                    data = json.loads(line)
                                    # Qwen tokens are under stats.models.<model>.tokens
                                    stats = data.get('stats', {})
                                    models = stats.get('models', {})
                                    for model, mdata in models.items():
                                        tokens = mdata.get('tokens', {})
                                        if tokens:
                                            inp = tokens.get('prompt', 0)
                                            out = tokens.get('candidates', 0)
                                            # Track max per model (cumulative stats)
                                            key = model
                                            if key not in model_max:
                                                model_max[key] = {'input': 0, 'output': 0}
                                            model_max[key]['input'] = max(model_max[key]['input'], inp)
                                            model_max[key]['output'] = max(model_max[key]['output'], out)
                                except json.JSONDecodeError:
                                    continue
                        # Add max values to daily stats
                        for m, vals in model_max.items():
                            self.daily_stats[date]['qwen_code']['input'] += vals['input']
                            self.daily_stats[date]['qwen_code']['output'] += vals['output']
                    else:
                        with open(filepath, 'r') as f:
                            data = json.load(f)
                        for msg in data.get('messages', []):
                            tokens = msg.get('tokens', {})
                            self.daily_stats[date]['qwen_code']['input'] += tokens.get('prompt', 0)
                            self.daily_stats[date]['qwen_code']['output'] += tokens.get('candidates', 0)
                except Exception as e:
                    if os.getenv('DEBUG'):
                        print(f"Error processing {filepath}: {e}", file=sys.stderr)

    def analyze_warp(self):
        """Extract WARP token usage from captured GraphQL responses."""
        # Check both Splitrail captured data and potential local WARP data
        patterns = [
            f"{self.home}/Projects/splitrail/schemas/warp/captured/graphql_responses_*.jsonl",
        ]

        # Track seen conversations to avoid duplicates (each API call returns ALL convos)
        seen_conversations = {}

        for pattern in patterns:
            for filepath in glob(pattern):
                try:
                    with open(filepath, 'r') as f:
                        for line in f:
                            try:
                                data = json.loads(line)
                                if data.get('operation') != 'GetConversationUsage':
                                    continue

                                # Navigate to conversation usage data
                                response = data.get('response', {})
                                user_data = response.get('data', {}).get('user', {}).get('user', {})
                                conversations = user_data.get('conversationUsage', [])

                                for conv in conversations:
                                    conv_id = conv.get('conversationId')
                                    if not conv_id:
                                        continue

                                    # Use lastUpdated as the date for this conversation
                                    last_updated = conv.get('lastUpdated', '')
                                    date = last_updated[:10] if last_updated else None
                                    if not date:
                                        continue

                                    usage = conv.get('usageMetadata', {})
                                    token_usage = usage.get('tokenUsage', [])

                                    total_tokens = sum(m.get('totalTokens', 0) for m in token_usage)

                                    # Only update if this is newer data for this conversation
                                    if conv_id not in seen_conversations or total_tokens > seen_conversations[conv_id]['total']:
                                        seen_conversations[conv_id] = {
                                            'date': date,
                                            'total': total_tokens
                                        }
                            except json.JSONDecodeError:
                                continue
                except Exception as e:
                    if os.getenv('DEBUG'):
                        print(f"Error processing {filepath}: {e}", file=sys.stderr)

        # Now aggregate by date
        for conv_id, data in seen_conversations.items():
            total = data['total']
            date = data['date']
            # WARP only provides total tokens, estimate 25% input / 75% output (AI-heavy)
            estimated_input = int(total * 0.25)
            estimated_output = int(total * 0.75)
            self.daily_stats[date]['warp']['input'] += estimated_input
            self.daily_stats[date]['warp']['output'] += estimated_output

    def analyze_cline(self):
        """Extract Cline token usage from VSCode extension data."""
        vscode_forks = ["Code", "Cursor", "Windsurf", "VSCodium", "Positron"]
        patterns = []

        for fork in vscode_forks:
            # macOS paths
            patterns.append(f"{self.home}/Library/Application Support/{fork}/User/globalStorage/saoudrizwan.claude-dev/tasks/*/ui_messages.json")
            # Linux paths
            patterns.append(f"{self.home}/.config/{fork}/User/globalStorage/saoudrizwan.claude-dev/tasks/*/ui_messages.json")

        for pattern in patterns:
            for filepath in glob(pattern):
                try:
                    with open(filepath, 'r') as f:
                        data = json.load(f)

                    mtime = os.path.getmtime(filepath)
                    date = datetime.fromtimestamp(mtime).strftime('%Y-%m-%d')

                    # Cline data is a flat list like Kilo Code
                    messages = data if isinstance(data, list) else data.get('messages', [])

                    for msg in messages:
                        text = msg.get('text', '')

                        # Extract embedded JSON tokens (similar to Kilo Code format)
                        in_match = re.search(r'"tokensIn":(\d+)', text)
                        out_match = re.search(r'"tokensOut":(\d+)', text)

                        if in_match:
                            self.daily_stats[date]['cline']['input'] += int(in_match.group(1))
                        if out_match:
                            self.daily_stats[date]['cline']['output'] += int(out_match.group(1))
                except Exception as e:
                    if os.getenv('DEBUG'):
                        print(f"Error processing {filepath}: {e}", file=sys.stderr)

    def run_all(self, tools=None):
        """Run analysis for specified or all tools."""
        if tools is None:
            tools = self.TOOLS

        analyzers = {
            'claude_code': self.analyze_claude_code,
            'codex_cli': self.analyze_codex_cli,
            'gemini_cli': self.analyze_gemini_cli,
            'opencode': self.analyze_opencode,
            'kilo_code': self.analyze_kilo_code,
            'qwen_code': self.analyze_qwen_code,
            'warp': self.analyze_warp,
            'cline': self.analyze_cline,
        }

        for tool in tools:
            if tool in analyzers:
                analyzers[tool]()

        return self.daily_stats

    def aggregate_by_tool(self):
        """Aggregate daily stats by tool."""
        tool_totals = defaultdict(lambda: {
            'input': 0,
            'output': 0,
            'total': 0,
            'max_daily_input': 0,
            'max_daily_output': 0,
            'max_daily_total': 0,
            'active_days': 0
        })
        tool_daily = defaultdict(list)

        for date, tools in self.daily_stats.items():
            for tool, tokens in tools.items():
                inp = tokens['input']
                out = tokens['output']
                total = inp + out

                tool_totals[tool]['input'] += inp
                tool_totals[tool]['output'] += out
                tool_totals[tool]['total'] += total

                tool_daily[tool].append({
                    'date': date,
                    'input': inp,
                    'output': out,
                    'total': total
                })

        for tool in tool_totals:
            daily = tool_daily[tool]
            if daily:
                tool_totals[tool]['max_daily_input'] = max(d['input'] for d in daily)
                tool_totals[tool]['max_daily_output'] = max(d['output'] for d in daily)
                tool_totals[tool]['max_daily_total'] = max(d['total'] for d in daily)
                tool_totals[tool]['active_days'] = len(daily)

        return dict(tool_totals)

    def generate_report_text(self, tool_totals):
        """Generate text report."""
        lines = []
        lines.append("=" * 90)
        lines.append("SPLITRAIL TOKEN USAGE ANALYSIS REPORT")
        lines.append(f"Generated: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
        lines.append("=" * 90)

        # Summary table
        lines.append(f"\n{'Tool':<15} {'Total Input':>15} {'Total Output':>15} {'Max Daily Out':>15} {'Days':>6}")
        lines.append("-" * 70)

        grand_input = 0
        grand_output = 0

        for tool in sorted(tool_totals.keys()):
            totals = tool_totals[tool]
            inp = totals['input']
            out = totals['output']
            max_out = totals['max_daily_output']
            days = totals['active_days']

            grand_input += inp
            grand_output += out

            lines.append(f"{tool:<15} {inp:>15,} {out:>15,} {max_out:>15,} {days:>6}")

        lines.append("-" * 70)
        lines.append(f"{'TOTAL':<15} {grand_input:>15,} {grand_output:>15,}")

        if grand_output > 0:
            ratio = grand_input / grand_output
            lines.append(f"\nInput:Output Ratio: {ratio:.1f}:1")

        # Max daily totals
        lines.append("\n" + "=" * 90)
        lines.append("MAX DAILY TOKENS BY TOOL")
        lines.append("=" * 90)
        lines.append(f"\n{'Tool':<15} {'Max Input':>15} {'Max Output':>15} {'Max Total':>15}")
        lines.append("-" * 60)

        for tool in sorted(tool_totals.keys()):
            totals = tool_totals[tool]
            lines.append(f"{tool:<15} {totals['max_daily_input']:>15,} {totals['max_daily_output']:>15,} {totals['max_daily_total']:>15,}")

        return "\n".join(lines)

    def generate_report_json(self, tool_totals):
        """Generate JSON report."""
        return json.dumps({
            'generated': datetime.now().isoformat(),
            'tools': tool_totals,
            'summary': {
                'total_input': sum(t['input'] for t in tool_totals.values()),
                'total_output': sum(t['output'] for t in tool_totals.values()),
                'max_daily_output': sum(t['max_daily_output'] for t in tool_totals.values()),
            }
        }, indent=2)

    def generate_report_csv(self, tool_totals):
        """Generate CSV report."""
        lines = ['tool,total_input,total_output,max_daily_input,max_daily_output,max_daily_total,active_days']
        for tool in sorted(tool_totals.keys()):
            t = tool_totals[tool]
            lines.append(f"{tool},{t['input']},{t['output']},{t['max_daily_input']},{t['max_daily_output']},{t['max_daily_total']},{t['active_days']}")
        return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description='Analyze token usage across AI coding tools')
    parser.add_argument('--json', action='store_true', help='Output as JSON')
    parser.add_argument('--csv', action='store_true', help='Output as CSV')
    parser.add_argument('--tool', type=str, help='Analyze specific tool only')
    parser.add_argument('--debug', action='store_true', help='Show debug output')
    args = parser.parse_args()

    if args.debug:
        os.environ['DEBUG'] = '1'

    analyzer = TokenAnalyzer()

    tools = [args.tool] if args.tool else None
    analyzer.run_all(tools)

    tool_totals = analyzer.aggregate_by_tool()

    if args.json:
        print(analyzer.generate_report_json(tool_totals))
    elif args.csv:
        print(analyzer.generate_report_csv(tool_totals))
    else:
        print(analyzer.generate_report_text(tool_totals))


if __name__ == "__main__":
    main()
