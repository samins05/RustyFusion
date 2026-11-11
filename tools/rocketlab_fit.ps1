<#
.SYNOPSIS
  Benchmarks rocket blast-range rules against what the game client actually hit (rocketlab.csv).

.DESCRIPTION
  Every CSV row is one mob near one rocket explosion: where it was relative to the explosion,
  how big its body is, and whether the game client said the rocket hit it.

  Every rule is recomputed from the raw columns (d2, dz, r, h, scale, body_r, body_h, weapon),
  so old and new CSV rows score the same way. The CSV's own `limit` / `verdict` columns
  are ignored (their meaning changed when the server's check changed).

  Rules benchmarked:
    PR #8        3D distance from the explosion to the mob's feet <= 300, same for every mob
    Radius       3D distance to the feet <= blast + m_iRadius (the obvious "blast touches the body" rule)
    Radius+      3D distance to the feet <= 300 + m_iRadius x m_fScale (a "generous" version of it)
    Old fit      our earlier hand-tuned rule: flat distance <= 250 + min(body_r x 150%, 280),
                 explosion within 150 below the feet / above the head (body_r/body_h include m_fScale)
    Client       the decompiled client's rule (NpcContainer.GetNearListMobTeam):
                 |explosion - (feet + h/2)| < max(r, h/2) + weapon m_iEffectArea   (raw r/h, no scale)
    Client+slack the same plus rocket_blast_slack from config.toml (what the server warns on)

  Scored on:
    Agrees with client  same hit / not-hit answer as the game (higher is better)
    False alarms        the client hit the mob, but the rule would flag it as cheating (want 0)
    Free cheat room     the client did NOT hit the mob, but the rule would still accept it (want low)

  Idle testbed mobs (exact positions) and moving mobs are scored separately.

.EXAMPLE
  .\tools\rocketlab_fit.ps1
  .\tools\rocketlab_fit.ps1 -Csv rocketlab-1103-1533.csv
#>
param(
    [string]$Csv = "rocketlab.csv",
    [double]$MovingThreshold = 50,
    [string]$Config = "config.toml",
    [string]$Xdt = "tabledata\xdt.json"
)

if (-not (Test-Path $Csv)) { Write-Host "No data yet: $Csv not found." -ForegroundColor Yellow; return }

# ------------------------------------------------------------------ drawing helpers
$H = [string][char]0x2500; $V = [string][char]0x2502
$TL = [char]0x250C; $TR = [char]0x2510; $BL = [char]0x2514; $BR = [char]0x2518
$TT = [char]0x252C; $BT = [char]0x2534; $LT = [char]0x251C; $RT = [char]0x2524; $X = [char]0x253C

function Write-Title([string]$text) {
    $w = $text.Length + 4
    Write-Host ""
    Write-Host ("$TL" + ($H * $w) + "$TR") -ForegroundColor DarkCyan
    Write-Host "$V  " -ForegroundColor DarkCyan -NoNewline
    Write-Host $text -ForegroundColor Cyan -NoNewline
    Write-Host "  $V" -ForegroundColor DarkCyan
    Write-Host ("$BL" + ($H * $w) + "$BR") -ForegroundColor DarkCyan
}

# $cols: array of @{ title; width; align }   $rows: array of arrays of @{ text; color }
function Write-Table($caption, $cols, $rows) {
    if ($caption) { Write-Host ""; Write-Host " $caption" -ForegroundColor White }
    $line = { param($l, $m, $r) "$l" + (($cols | ForEach-Object { $H * ($_.width + 2) }) -join "$m") + "$r" }
    Write-Host (& $line $TL $TT $TR) -ForegroundColor DarkGray
    Write-Host "$V" -ForegroundColor DarkGray -NoNewline
    foreach ($c in $cols) {
        Write-Host (" " + $c.title.PadRight($c.width) + " ") -ForegroundColor Gray -NoNewline
        Write-Host "$V" -ForegroundColor DarkGray -NoNewline
    }
    Write-Host ""
    Write-Host (& $line $LT $X $RT) -ForegroundColor DarkGray
    foreach ($row in $rows) {
        Write-Host "$V" -ForegroundColor DarkGray -NoNewline
        for ($i = 0; $i -lt $cols.Count; $i++) {
            $cell = $row[$i]; $c = $cols[$i]
            $t = [string]$cell.text
            if ($t.Length -gt $c.width) { $t = $t.Substring(0, $c.width) }
            $t = if ($c.align -eq 'right') { $t.PadLeft($c.width) } else { $t.PadRight($c.width) }
            $color = if ($cell.color) { $cell.color } else { 'White' }
            Write-Host " $t " -ForegroundColor $color -NoNewline
            Write-Host "$V" -ForegroundColor DarkGray -NoNewline
        }
        Write-Host ""
    }
    Write-Host (& $line $BL $BT $BR) -ForegroundColor DarkGray
}

function Cell($text, $color = 'White') { @{ text = $text; color = $color } }
function Pct($n, $d) { if ($d -gt 0) { 100.0 * $n / $d } else { 0 } }
function Col($title, $width, $align = 'left') { @{ title = $title; width = $width; align = $align } }

# ------------------------------------------------------------------ config + weapon blast radius
$slack = 25
if (Test-Path $Config) {
    $m = [regex]::Match((Get-Content $Config -Raw), "(?m)^\s*rocket_blast_slack\s*=\s*(\d+)")
    if ($m.Success) { $slack = [double]$m.Groups[1].Value }
}

$csvRows = Import-Csv $Csv
$weapons = @($csvRows | Select-Object -ExpandProperty weapon -Unique)
$effectArea = @{}
if (Test-Path $Xdt) {
    # (PowerShell names are case-insensitive: don't call this $xdt, it would clobber -Xdt)
    $tableRoot = Get-Content $Xdt -Raw | ConvertFrom-Json
    foreach ($it in $tableRoot.m_pWeaponItemTable.m_pItemData) {
        $id = [string]$it.m_iItemNumber
        if ($weapons -contains $id) { $effectArea[$id] = [double]$it.m_iEffectArea }
    }
}
foreach ($w in $weapons) {
    if (-not $effectArea.ContainsKey($w)) {
        Write-Host "  Weapon $w not found in $Xdt; assuming blast radius 200." -ForegroundColor Yellow
        $effectArea[$w] = 200.0
    }
}

# ------------------------------------------------------------------ load data
$names = @{ '2463' = 'Stalactitan'; '121' = 'Timber Terror'; '438' = 'Stone Drake'; '593' = 'Bigger Brute'
            '600' = 'Elephantine Enforcer'; '2097' = 'Raging Fossil Crawler' }
function MobName($type) { if ($names.ContainsKey($type)) { $names[$type] } else { "type $type" } }

$rows = $csvRows | ForEach-Object {
    $rad = [double]$_.r; $hgt = [double]$_.h
    $d2 = [double]$_.d2; $dz = [double]$_.dz           # dz = mob feet z - explosion z
    $vz = $dz + $hgt / 2                                # explosion -> body center, vertical
    [pscustomobject]@{
        shot = $_.shot; ts = $_.ts_ms; type = $_.type
        summoned = $_.summoned -eq 'true'; dead = $_.dead -eq 'true'; drift = [double]$_.drift
        hit = $_.claimed -eq 'true'
        d2 = $d2; d3 = [double]$_.d3; up = -$dz
        r = $rad; h = $hgt; body_r = [double]$_.body_r; body_h = [double]$_.body_h
        cdist = [Math]::Sqrt($d2 * $d2 + $vz * $vz)     # client: distance to body center
        csize = [Math]::Max($rad, $hgt / 2)             # client: body size
        blast = $effectArea[$_.weapon]
    }
}
$idle = @($rows | Where-Object { $_.summoned -and -not $_.dead -and $_.drift -le $MovingThreshold })
$moving = @($rows | Where-Object { $_.summoned -and -not $_.dead -and $_.drift -gt $MovingThreshold })
$shots = @($rows | ForEach-Object { "$($_.ts.Substring(0, [Math]::Min(8, $_.ts.Length)))-$($_.shot)" } | Select-Object -Unique).Count

# ------------------------------------------------------------------ rules
# each rule: name, how, formula text, and a scriptblock returning { limit, distance } for a row
$rules = @(
    @{ name = 'PR #8'; how = '3D to feet'; text = '300 for every mob'
       eval = { param($x) @(300.0, $x.d3) } },
    # what a server dev would naturally try first with tabledata in hand: the blast sphere touches
    # the mob's radius, measured from the mob's position (its feet) like PR #8 does
    @{ name = 'Radius'; how = '3D to feet'; text = 'blast + r'
       eval = { param($x) @(($x.blast + $x.r), $x.d3) } },
    @{ name = 'Radius+'; how = '3D to feet'; text = '300 + r x scale  ("generous")'
       eval = { param($x) @((300 + $x.body_r), $x.d3) } },
    @{ name = 'Old fit'; how = 'flat+height'; text = '250 + min(body x 150%, 280), height +-150'
       eval = { param($x)
           $lim = 250 + [Math]::Min($x.body_r * 1.5, 280)
           $inHeight = $x.up -ge -150 -and $x.up -le $x.body_h + 150
           # outside the height window counts as infinitely far
           @($lim, $(if ($inHeight) { $x.d2 } else { [double]::PositiveInfinity })) } },
    @{ name = 'Client'; how = '3D to center'; text = 'max(r, h/2) + blast'
       eval = { param($x) @(($x.csize + $x.blast), $x.cdist) } },
    @{ name = 'Client+slack'; how = '3D to center'; text = "max(r, h/2) + blast + $slack (config)"
       eval = { param($x) @(($x.csize + $x.blast + $slack), $x.cdist) } }
)
# client rule is strict "<"; the others are "<="
function Passes($row, $r) {
    $e = & $r.eval $row
    if ($r.name -like 'Client*') { $e[1] -lt $e[0] } else { $e[1] -le $e[0] }
}
function Margin($row, $r) { $e = & $r.eval $row; $e[0] - $e[1] }   # negative = beyond the limit

# returns falseAlarms, realHits, freeRoom, notHit
function Score($data, $r) {
    $fa = 0; $nh = 0; $fr = 0; $nn = 0
    foreach ($x in $data) {
        $p = Passes $x $r
        if ($x.hit) { $nh++; if (-not $p) { $fa++ } } else { $nn++; if ($p) { $fr++ } }
    }
    , @($fa, $nh, $fr, $nn)
}

# ------------------------------------------------------------------ header
Write-Title "Rocket range check benchmark"
Write-Host ("  {0} rocket hits, {1} mob checks  ({2} idle mobs, {3} moving mobs)" -f $shots, $rows.Count, $idle.Count, $moving.Count) -ForegroundColor Gray
Write-Host ("  Weapons: " + (($weapons | ForEach-Object { "$_ (blast $($effectArea[$_]))" }) -join ', ')) -ForegroundColor Gray
Write-Host "  Mobs the game client hit are treated as legit." -ForegroundColor Gray
if (-not @($idle | Where-Object hit).Count -and -not @($moving | Where-Object hit).Count) {
    Write-Host "  Not enough data yet: shoot some testbed mobs first." -ForegroundColor Yellow; return
}

Write-Host ""
Write-Host " How to read this" -ForegroundColor White
Write-Host "   Agrees with client   " -ForegroundColor Cyan -NoNewline; Write-Host "how often the rule gives the same hit/not-hit answer as the game (higher is better)" -ForegroundColor Gray
Write-Host "   False alarms         " -ForegroundColor Cyan -NoNewline; Write-Host "real hits the rule would flag as cheating (want 0)" -ForegroundColor Gray
Write-Host "   Free cheat room      " -ForegroundColor Cyan -NoNewline; Write-Host "mobs the game did NOT hit that the rule would still accept (want low)" -ForegroundColor Gray

# ------------------------------------------------------------------ overall benchmark
function Write-Overall($caption, $data) {
    if (-not $data.Count) { return }
    $cols = @((Col 'Rule' 12), (Col 'Measured' 12), (Col 'Allowed distance' 40), (Col 'Agrees with client' 18 'right'),
              (Col 'False alarms' 16 'right'), (Col 'Free cheat room' 16 'right'))
    $tbl = foreach ($r in $rules) {
        $s = Score $data $r
        $agree = Pct ($s[1] - $s[0] + $s[3] - $s[2]) ($s[1] + $s[3])
        $agreeColor = if ($agree -ge 99) { 'Green' } elseif ($agree -ge 95) { 'Yellow' } else { 'Red' }
        $faPct = Pct $s[0] $s[1]
        $faColor = if ($s[0] -eq 0) { 'Green' } elseif ($faPct -le 2) { 'Yellow' } else { 'Red' }
        $frPct = Pct $s[2] $s[3]
        $frColor = if ($frPct -le 2) { 'Green' } elseif ($frPct -le 10) { 'Yellow' } else { 'Red' }
        , @((Cell $r.name 'White'), (Cell $r.how 'Gray'), (Cell $r.text 'Gray'),
            (Cell ("{0:N1}%" -f $agree) $agreeColor),
            (Cell ("{0} of {1}" -f $s[0], $s[1]) $faColor),
            (Cell ("{0} of {1}" -f $s[2], $s[3]) $frColor))
    }
    Write-Table $caption $cols $tbl
}
Write-Overall "Idle mobs (exact positions: tests the rule itself)" $idle
Write-Overall "Moving mobs (client and server positions differ: tests the slack)" $moving

# ------------------------------------------------------------------ per mob type (client rule)
$client = $rules | Where-Object { $_.name -eq 'Client' }
$pr8 = $rules | Where-Object { $_.name -eq 'PR #8' }
$cols2 = @((Col 'Mob' 22), (Col 'r' 4 'right'), (Col 'h' 5 'right'), (Col 'Client limit' 12 'right'),
           (Col 'Real hits' 9 'right'), (Col 'Closest call' 12 'right'), (Col 'Client false alarms' 19 'right'), (Col 'PR #8 false alarms' 18 'right'))
$alive = @($idle) + @($moving)
$tbl2 = foreach ($g in ($alive | Group-Object type | Sort-Object { $_.Group[0].csize })) {
    $x0 = $g.Group[0]
    $hits = @($g.Group | Where-Object hit)
    if (-not $hits.Count) { continue }
    $closest = ($hits | ForEach-Object { Margin $_ $client } | Measure-Object -Minimum).Minimum
    $fa = @($hits | Where-Object { -not (Passes $_ $client) }).Count
    $fa8 = @($hits | Where-Object { -not (Passes $_ $pr8) }).Count
    $closeColor = if ($closest -lt 0) { 'Red' } elseif ($closest -lt 25) { 'Yellow' } else { 'Green' }
    , @((Cell (MobName $g.Name)), (Cell $x0.r 'Gray'), (Cell $x0.h 'Gray'),
        (Cell ("{0:N0}" -f ($x0.csize + $x0.blast)) 'Cyan'), (Cell $hits.Count 'Gray'),
        (Cell ("{0:+0;-0}" -f $closest) $closeColor),
        (Cell $fa $(if ($fa) { 'Red' } else { 'Green' })),
        (Cell $fa8 $(if ($fa8) { 'Red' } else { 'Green' })))
}
Write-Table "Per mob type, idle + moving  (Closest call = smallest margin of any real hit under the client rule; negative = flagged)" $cols2 $tbl2

# ------------------------------------------------------------------ slack recommendation
$overs = @($moving + $idle | Where-Object hit | ForEach-Object { -(Margin $_ $client) } | Where-Object { $_ -gt 0 } | Sort-Object)
Write-Host ""
Write-Host " Slack (rocket_blast_slack, currently $slack)" -ForegroundColor White
if (-not $overs.Count) {
    Write-Host "   Every real hit is inside the client rule with no slack." -ForegroundColor Green
} else {
    Write-Host ("   Real hits beyond the client rule: {0}, by {1}" -f $overs.Count, (($overs | ForEach-Object { "{0:N0}" -f $_ }) -join ', ')) -ForegroundColor Gray
    $covered = @($overs | Where-Object { $_ -lt $slack }).Count
    $color = if ($covered -eq $overs.Count) { 'Green' } else { 'Yellow' }
    Write-Host ("   Slack $slack covers {0} of {1}. The rest are movement lag; raise slack only if they keep showing up." -f $covered, $overs.Count) -ForegroundColor $color
}
Write-Host ""
