# @vornrun/connector-kusto

Trigger Vorn workflows from the rows an Azure Data Explorer (Kusto) query returns.

Point a Vorn MCP connection at this package, give it a KQL query, and every new
row the query produces becomes a workflow trigger.

## Install

Nothing to install by hand. In Vorn, create an **MCP** connection with:

| Field   | Value                                |
| ------- | ------------------------------------ |
| Command | `npx`                                |
| Args    | `["-y", "@vornrun/connector-kusto"]` |

## Authentication

Kusto has no personal access token — every request is authenticated with
Microsoft Entra ID. The connector uses `DefaultAzureCredential`, so it picks up
whichever credential the machine already has:

1. A service principal from `AZURE_CLIENT_ID` / `AZURE_TENANT_ID` / `AZURE_CLIENT_SECRET`
2. Workload identity or a managed identity, when running on Azure
3. Your own signed-in user, from `az login`

For local use, `az login` is all that is needed. Tokens are cached for the life
of the process and refreshed five minutes before they expire.

## Configuration

| Variable                 | Required | Default     | Meaning                                                    |
| ------------------------ | -------- | ----------- | ---------------------------------------------------------- |
| `KUSTO_CLUSTER`          | yes      |             | `help`, `adx.eastus.kusto.windows.net`, or a full URL      |
| `KUSTO_DATABASE`         | yes      |             | Database to query                                          |
| `KUSTO_QUERY`            | yes      |             | The KQL to poll                                            |
| `KUSTO_ID_COLUMN`        | no       | `Id`        | Stable per-row id. Vorn dedupes on it.                     |
| `KUSTO_TIMESTAMP_COLUMN` | no       | `Timestamp` | Column the poll watermark advances from                    |
| `KUSTO_TITLE_COLUMN`     | no       | `Title`     | Falls back to the id when absent                           |
| `KUSTO_URL_COLUMN`       | no       |             | Link to open from the event                                |
| `KUSTO_LOOKBACK`         | no       | `1h`        | How far back the very first poll looks (`30m`, `2h`, `7d`) |

## Writing the query

Two parameters are declared for you. Reference them to keep each poll bounded:

- `vorn_since` (`datetime`) — the watermark. On the first poll this is
  `now() - KUSTO_LOOKBACK`, so connecting a trigger never replays the whole table.
- `vorn_limit` (`long`) — the page size Vorn asked for.

```kql
Alerts
| where FiredAt >= vorn_since
| where Severity <= 2
| project Id = AlertId, Timestamp = FiredAt, Title = Summary, Severity, Host
| order by Timestamp asc
| take vorn_limit
```

Both are bound as real KQL query parameters, never pasted into the query text.

Do not add your own `declare query_parameters` statement — KQL allows only one,
and the connector supplies it.

Every column you project is available to workflow templates as
`{{trigger.item.Host}}`, `{{trigger.item.Severity}}`, and so on.

### Dedupe

The trigger uses the SDK's `timestamp` strategy. Rows already delivered are
dropped even when the query returns them again, including the awkward case
where several rows share the newest timestamp. Returning slightly too much is
therefore safe; returning too little loses events.

Kusto datetimes come back without a zone. They are normalized to ISO 8601 UTC
so the watermark compares correctly.

## Action: `runQuery`

Run a read-only query from a workflow step.

| Input      | Required | Meaning                      |
| ---------- | -------- | ---------------------------- |
| `query`    | yes      | KQL to run                   |
| `database` | no       | Defaults to `KUSTO_DATABASE` |

Returns `rowCount`, `columns`, and `rows` (each row keyed by column name).
Queries cannot mutate data, so the step is safe to retry.

## Troubleshooting

**`No Azure credential was available`** — run `az login`, or set the service
principal variables above.

**`Query result has no "Id" column`** — the query did not project the id
column. Either `project Id = ...` or set `KUSTO_ID_COLUMN`.

**Events arrive repeatedly** — the query is probably ignoring `vorn_since`.
Dedupe still suppresses them, but each poll transfers the full result set.
