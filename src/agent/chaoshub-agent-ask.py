"""chaoshub-agent-ask: asks pr-agent a question and prints the answer.

chaoshub writes this file to its state directory and runs it with the
Python of the pr-agent command. With publish_output=false, the ask tool
of pr-agent keeps the answer in its settings (data.answer) and prints it
nowhere. This script runs the tool in its own settings scope, as
pr_agent/cli.py does, and prints the answer to stdout. The log goes to
stderr.

Usage: chaoshub-agent-ask <pr_url> <question>
Exit code: 0 with an answer, 1 without an answer, 2 for bad arguments.
"""

import asyncio
import copy
import os
import sys

from loguru import logger
from starlette_context import request_cycle_context

from pr_agent.agent.pr_agent import PRAgent
from pr_agent.config_loader import get_settings, global_settings


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    pr_url, question = sys.argv[1], sys.argv[2]
    # stdout is for the answer only.
    logger.remove()
    logger.add(sys.stderr, level=os.environ.get("LOG_LEVEL", "INFO"), colorize=False)
    with request_cycle_context({"settings": copy.deepcopy(global_settings)}):
        get_settings().set("CONFIG.CLI_MODE", True)
        result = asyncio.run(PRAgent().handle_request(pr_url, ["ask", question]))
        answer = get_settings().get("data.answer")
    if result is not True or not answer:
        print("pr-agent gave no answer", file=sys.stderr)
        return 1
    print(answer)
    return 0


if __name__ == "__main__":
    sys.exit(main())
