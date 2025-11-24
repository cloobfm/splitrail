#!/usr/bin/env python3
"""
Warp GraphQL Interceptor for mitmproxy
Captures and logs GraphQL responses from app.warp.dev
"""

from mitmproxy import http
import json
from datetime import datetime
import os

# Output directory for captured data
OUTPUT_DIR = os.path.expanduser("~/Projects/splitrail/schemas/warp/captured")
os.makedirs(OUTPUT_DIR, exist_ok=True)

# Log files
GRAPHQL_LOG = os.path.join(OUTPUT_DIR, f"graphql_responses_{datetime.now().strftime('%Y%m%d')}.jsonl")
USAGE_LOG = os.path.join(OUTPUT_DIR, f"usage_data_{datetime.now().strftime('%Y%m%d')}.jsonl")

def response(flow: http.HTTPFlow) -> None:
    """Intercept responses from Warp's GraphQL API"""

    # Only process Warp GraphQL requests
    if "app.warp.dev/graphql" not in flow.request.pretty_url:
        return

    try:
        # Parse the response
        if flow.response and flow.response.text:
            response_data = json.loads(flow.response.text)

            # Get operation name from URL query params or request body
            operation_name = "Unknown"
            if flow.request.query.get("op"):
                operation_name = flow.request.query["op"]

            # Create log entry with full context
            log_entry = {
                "timestamp": datetime.utcnow().isoformat() + "Z",
                "operation": operation_name,
                "url": flow.request.pretty_url,
                "request_id": flow.request.headers.get("x-request-id", ""),
                "response": response_data
            }

            # Log all GraphQL responses
            with open(GRAPHQL_LOG, "a") as f:
                f.write(json.dumps(log_entry) + "\n")

            # Extract and log specific usage information
            if "data" in response_data:
                data = response_data["data"]

                # Extract user/usage data if present
                if "user" in data:
                    user_data = data["user"]

                    # Check for UserOutput type
                    if isinstance(user_data, dict) and user_data.get("__typename") == "UserOutput":
                        user_info = user_data.get("user", {})

                        # Extract request limit info (credits, usage)
                        if "requestLimitInfo" in user_info:
                            usage_entry = {
                                "timestamp": datetime.utcnow().isoformat() + "Z",
                                "type": "request_limit_info",
                                "data": user_info["requestLimitInfo"]
                            }

                            with open(USAGE_LOG, "a") as f:
                                f.write(json.dumps(usage_entry) + "\n")

                        # Extract bonus grants info
                        if "bonusGrants" in user_info:
                            usage_entry = {
                                "timestamp": datetime.utcnow().isoformat() + "Z",
                                "type": "bonus_grants",
                                "data": user_info["bonusGrants"]
                            }

                            with open(USAGE_LOG, "a") as f:
                                f.write(json.dumps(usage_entry) + "\n")

                        # Extract workspace info
                        if "workspaces" in user_info:
                            workspaces = user_info["workspaces"]

                            # Feature model choices (available models)
                            if "featureModelChoice" in workspaces:
                                usage_entry = {
                                    "timestamp": datetime.utcnow().isoformat() + "Z",
                                    "type": "feature_model_choices",
                                    "data": workspaces["featureModelChoice"]
                                }

                                with open(USAGE_LOG, "a") as f:
                                    f.write(json.dumps(usage_entry) + "\n")

                            # Bonus grants info from workspaces
                            if "bonusGrantsInfo" in workspaces:
                                usage_entry = {
                                    "timestamp": datetime.utcnow().isoformat() + "Z",
                                    "type": "workspace_bonus_grants",
                                    "data": workspaces["bonusGrantsInfo"]
                                }

                                with open(USAGE_LOG, "a") as f:
                                    f.write(json.dumps(usage_entry) + "\n")

            print(f"[{datetime.now().strftime('%H:%M:%S')}] Captured: {operation_name}")

    except json.JSONDecodeError:
        # Response wasn't JSON, skip it
        pass
    except Exception as e:
        print(f"Error processing response: {e}")


def request(flow: http.HTTPFlow) -> None:
    """Log GraphQL requests for correlation"""

    if "app.warp.dev/graphql" not in flow.request.pretty_url:
        return

    try:
        operation_name = flow.request.query.get("op", "Unknown")
        print(f"[{datetime.now().strftime('%H:%M:%S')}] Request: {operation_name}")
    except Exception as e:
        print(f"Error logging request: {e}")
