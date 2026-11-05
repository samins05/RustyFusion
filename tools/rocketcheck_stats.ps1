<#
.SYNOPSIS
  Averages [rocketcheck] margins in chunks of N entries, to gauge the rocket blast-range formula.

.DESCRIPTION
  Server rule (client's own rule, see BlastCheck in combat.rs):
    limit = body + blast + slack, where body = max(m_iRadius, m_iHeight / 2), blast = weapon m_iEffectArea
    dist  = 3D distance from the explosion to the mob's body center (feet + m_iHeight / 2)
  margin = limit - dist (positive = inside the limit).
    OK    claimed, inside limit      (want: small avg margin = tight but no false warnings)
    OVER  claimed, beyond limit      (want: none; would warn)
    LOOSE unclaimed, inside limit    (want: few, small margin; this is overshoot)
    NEAR  unclaimed, just beyond     (rule agrees with client)

  Pass -Slack to re-score the logged dist/body/blast values with a different slack, without
  restarting the server. Lines from before 2026-11-04 use the old format
  (body= d2= limit= height= allowed=, flat+height rule): they're shown as logged and skipped
  when re-scoring.

.EXAMPLE
  .\tools\rocketcheck_stats.ps1                          # whole hybrid.log, chunks of 20
  .\tools\rocketcheck_stats.ps1 -Every 50 -ByType        # also break each chunk down per mob type
  .\tools\rocketcheck_stats.ps1 -Slack 0                 # what-if with no movement slack
  .\tools\rocketcheck_stats.ps1 -Follow                  # live: print a summary every N new entries
#>
param(
    [string]$Log = "hybrid.log",
    [int]$Every = 20,
    [Nullable[double]]$Slack,
    [switch]$ByType,
    [switch]$Follow
)

$rescore = $null -ne $Slack

function Parse-Line([string]$line) {
    if ($line -notmatch '\[rocketcheck\] (\w+)') { return $null }
    $verdict = $Matches[1]
    $get = { param($k) [regex]::Match($line, " $k=([^ ]+)").Groups[1].Value }
    $dist = & $get 'dist'
    $row = [pscustomobject]@{
        claimed = $verdict -in 'OK', 'OVER'
        type    = & $get 'type'
        body    = [double](& $get 'body')
        blast   = [double](& $get 'blast')
        dist    = if ($dist) { [double]$dist } else { $null }
        margin  = [double](& $get 'margin')
        verdict = $verdict
    }
    if ($rescore) {
        if ($null -eq $row.dist) { return $null }   # old-format line
        $limit = $row.body + $row.blast + $Slack
        $row.margin = $limit - $row.dist
        $row.verdict = if ($row.claimed) {
            if ($row.margin -ge 0) { 'OK' } else { 'OVER' }
        } else {
            if ($row.margin -ge 0) { 'LOOSE' } else { 'NEAR' }
        }
    }
    $row
}

function Write-Summary([string]$label, $rows) {
    $parts = foreach ($v in 'OK', 'OVER', 'LOOSE', 'NEAR') {
        $g = @($rows | Where-Object verdict -eq $v)
        if ($g.Count -eq 0) { "{0} 0" -f $v; continue }
        $m = $g.margin | Measure-Object -Average -Minimum -Maximum
        "{0} {1} avg {2:+0;-0} [{3:+0;-0}..{4:+0;-0}]" -f $v, $g.Count, $m.Average, $m.Minimum, $m.Maximum
    }
    "{0,-14} {1}" -f $label, ($parts -join " | ")
    if ($ByType) {
        foreach ($t in ($rows | Group-Object type | Sort-Object Name)) {
            $sub = foreach ($v in 'OK', 'OVER', 'LOOSE') {
                $g = @($t.Group | Where-Object verdict -eq $v)
                if ($g.Count -gt 0) { "{0} {1} avg {2:+0;-0}" -f $v, $g.Count, ($g.margin | Measure-Object -Average).Average }
            }
            "    type {0,-6} body {1,-4} {2}" -f $t.Name, $t.Group[0].body, ($sub -join " | ")
        }
    }
}

$formula = if ($rescore) { "limit = body + blast + $Slack (re-scored)" } else { "limit = body + blast + slack (as logged)" }
Write-Output $formula

if ($Follow) {
    $buf = New-Object System.Collections.Generic.List[object]
    $n = 0
    Get-Content $Log -Wait -Tail 0 | ForEach-Object {
        $r = Parse-Line $_
        if ($null -ne $r) {
            $buf.Add($r)
            if ($buf.Count -ge $Every) {
                $n++
                Write-Summary ("chunk {0}" -f $n) $buf
                $buf.Clear()
            }
        }
    }
    return
}

$all = @(Get-Content $Log | ForEach-Object { Parse-Line $_ } | Where-Object { $_ })
if ($all.Count -eq 0) { Write-Output "no [rocketcheck] lines in $Log"; return }
for ($i = 0; $i -lt $all.Count; $i += $Every) {
    $end = [Math]::Min($i + $Every, $all.Count) - 1
    Write-Summary ("{0}-{1}" -f ($i + 1), ($end + 1)) $all[$i..$end]
}
Write-Summary "TOTAL" $all
