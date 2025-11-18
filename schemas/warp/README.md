# Warp Schema

This schema describes the data captured from the Warp terminal's network log.

## Data Source

The data is parsed from the `warp_network.log` file located in the Warp application support directory.

-   **macOS**: `~/Library/Application Support/dev.warp.Warp-Stable/warp_network.log`

## Structure

The log file contains network requests made by the Warp client. The analyzer specifically looks for `POST` requests to the `/analytics/block` endpoint. The body of these requests is a JSON object that contains information about a command that was just executed.

Each command block is parsed into two `ConversationMessage` entries:

1.  A `User` message containing the command that was run.
2.  An `Assistant` message containing the output of the command.

The `was_autosuggestion_from_ai` field is used to determine if the command was AI-assisted.
