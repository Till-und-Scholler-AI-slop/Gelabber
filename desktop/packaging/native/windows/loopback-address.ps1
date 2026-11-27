# libwebrtc sends from the machine's address, which Windows may not route to
# 127.0.0.1: probes it, and moves the tests' server to the machine's address
# if needed (GELABBER_TEST_LISTEN_IP, written to GITHUB_ENV when it is set).
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
# Whether a datagram from a socket on $from reaches one on $to, and as coming from $from.
function Reaches($from, $to) {
  $outgoing = $null
  $receiver = $null
  try {
    $outgoing = [Net.Sockets.UdpClient]::new([Net.IPEndPoint]::new($from, 0))
    $receiver = [Net.Sockets.UdpClient]::new([Net.IPEndPoint]::new($to, 0))
    $receiver.Client.ReceiveTimeout = 2000
    $null = $outgoing.Send([byte[]]@(1), 1, [Net.IPEndPoint]$receiver.Client.LocalEndPoint)
    $source = [Net.IPEndPoint]::new([Net.IPAddress]::Any, 0)
    $null = $receiver.Receive([ref]$source)
    Write-Host "UDP from $from to ${to}: arrives, sent by $($source.Address)"
    return $source.Address.Equals($from)
  } catch {
    Write-Host "UDP from $from to ${to}: $($_.Exception.Message)"
    return $false
  } finally {
    if ($outgoing) { $outgoing.Dispose() }
    if ($receiver) { $receiver.Dispose() }
  }
}
# The address this machine sends from: a UDP socket takes it on when
# it is connected, without sending anything.
$own = $null
$route = [Net.Sockets.UdpClient]::new([Net.Sockets.AddressFamily]::InterNetwork)
try {
  $route.Connect('192.0.2.1', 9)
  $own = ([Net.IPEndPoint]$route.Client.LocalEndPoint).Address
} catch {
  Write-Host "no route to tell this machine's address by: $($_.Exception.Message)"
} finally {
  $route.Dispose()
}
$loopback = [Net.IPAddress]::Loopback
$listen = $null
if ($own -and -not [Net.IPAddress]::IsLoopback($own)) {
  $there = Reaches $own $loopback
  $back = Reaches $loopback $own
  if ($there -and $back) {
    $listen = $loopback
  } elseif (Reaches $own $own) {
    $listen = $own
  }
}
if (-not $listen) {
  '::warning::No address was found that this machine reaches from its own: the tests keep their server on 127.0.0.1'
} elseif ($listen.Equals($loopback)) {
  'the server of the tests stays on 127.0.0.1'
} else {
  "the server of the tests listens on ${listen}: 127.0.0.1 is out of reach from there"
  Add-Content $env:GITHUB_ENV "GELABBER_TEST_LISTEN_IP=$listen"
}
