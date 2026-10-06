'=========================================================================
' 特殊字符输入核心模块  SpecialCharInput.inc
'-------------------------------------------------------------------------
' 提供两个能力，供各个符号页的按钮调用：
'   1) InsertSymbol  把字符用 SendInput(Unicode) 模拟键盘输入到“当前焦点窗口”
'   2) CopySymbol    把字符复制到系统剪贴板
'
' 工作原理：
'   本工具主窗口设置了“置顶 + 不抢焦点 (WS_EX_NOACTIVATE)”，
'   因此当用户点击符号按钮时，键盘焦点仍然停留在用户原本打字的程序里，
'   InsertSymbol 就能把符号直接敲进那个程序，无需来回切换窗口。
'
' 说明：SendInput 使用 KEYEVENTF_UNICODE，可输入任意 Unicode 字符
'       （包括中文、假名、序号、制表符等），与键盘布局无关。
'=========================================================================

'-------------------------------------------------------------------------
' 把一段文本逐字符模拟键盘输入（支持全部 Unicode 字符）
'-------------------------------------------------------------------------
Sub InsertUnicodeText(ByVal s As CWSTR)
   Dim n As Long = Len(s)                 'UTF-16 码元个数（代理对按 2 个计）
   If n <= 0 Then Return
   Dim cnt As Long = n * 2                '每个码元：一次按下 + 一次抬起
   Dim p As INPUT_ Ptr = Callocate(cnt, SizeOf(INPUT_))
   If p = 0 Then Return
   Dim pc As USHORT Ptr = Cast(USHORT Ptr, s.vptr)
   Dim i As Long
   For i = 0 To n - 1
      '---- 按下 ----
      p[i*2].type = INPUT_KEYBOARD
      p[i*2].ki.wVk = 0
      p[i*2].ki.wScan = pc[i]
      p[i*2].ki.dwFlags = KEYEVENTF_UNICODE
      p[i*2].ki.time = 0
      p[i*2].ki.dwExtraInfo = 0
      '---- 抬起 ----
      p[i*2+1].type = INPUT_KEYBOARD
      p[i*2+1].ki.wVk = 0
      p[i*2+1].ki.wScan = pc[i]
      p[i*2+1].ki.dwFlags = KEYEVENTF_UNICODE Or KEYEVENTF_KEYUP
      p[i*2+1].ki.time = 0
      p[i*2+1].ki.dwExtraInfo = 0
   Next
   SendInput cnt, p, SizeOf(INPUT_)
   Deallocate p
End Sub

'-------------------------------------------------------------------------
' 把一段文本复制到剪贴板（CF_UNICODETEXT）
'-------------------------------------------------------------------------
Sub CopyUnicodeToClipboard(ByVal s As CWSTR)
   Dim n As Long = Len(s)
   If n <= 0 Then Return
   If OpenClipboard(0) = 0 Then Return
   EmptyClipboard
   Dim bytes As Long = (n + 1) * 2        '结尾含一个 0 字符
   Dim h As HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes)
   If h <> 0 Then
      Dim d As USHORT Ptr = Cast(USHORT Ptr, GlobalLock(h))
      If d <> 0 Then
         Dim src As USHORT Ptr = Cast(USHORT Ptr, s.vptr)
         Dim i As Long
         For i = 0 To n                  '复制 n+1 个码元（含 0 结尾）
            d[i] = src[i]
         Next
         GlobalUnlock h
         SetClipboardData(CF_UNICODETEXT, h)
         '成功后内存归系统所有，不可再释放
      Else
         GlobalFree h
      End If
   End If
   CloseClipboard
End Sub

'-------------------------------------------------------------------------
' 目标窗口（本工具以外、最近获得焦点的窗口）
' 目的：点击符号按钮后，SendInput 要发给“用户原本打字的程序”。
'       如果点击使本工具窗口被激活、焦点离开了目标程序，就先切回目标程序再发送。
'-------------------------------------------------------------------------
Dim Shared g_hInputTarget As HWND = 0

'记录目标窗口（由 Form1 在收到 WM_ACTIVATE 时调用，传入的那个窗口即“另一侧”的窗口）
Sub SetInputTarget(ByVal h As HWND)
   If h = 0 Then Return
   Dim pid As DWORD = 0
   GetWindowThreadProcessId h, @pid
   If pid = GetCurrentProcessId() Then Return   '自己的窗口不算目标
   g_hInputTarget = h
End Sub

'发送前：若当前焦点在本工具上，则把前台窗口切回目标程序
Sub EnsureInputTarget()
   Dim hfg As HWND = GetForegroundWindow()
   If hfg = 0 Then Return
   Dim pid As DWORD = 0
   GetWindowThreadProcessId hfg, @pid
   If pid <> GetCurrentProcessId() Then Return  '焦点已在别的程序上，直接发送即可
   If IsWindow(g_hInputTarget) = 0 Then Return
   If g_hInputTarget = hfg Then Return
   '用 AttachThreadInput 把输入队列连起来，才能可靠地切换前台/焦点
   Dim tidMe As DWORD = GetCurrentThreadId()
   Dim tidFg As DWORD = GetWindowThreadProcessId(hfg, Null)
   Dim tidTg As DWORD = GetWindowThreadProcessId(g_hInputTarget, Null)
   If tidFg <> 0 AndAlso tidFg <> tidMe Then AttachThreadInput tidMe, tidFg, True
   If tidTg <> 0 AndAlso tidTg <> tidMe Then AttachThreadInput tidMe, tidTg, True
   SetForegroundWindow g_hInputTarget
   SetFocus g_hInputTarget
   If tidTg <> 0 AndAlso tidTg <> tidMe Then AttachThreadInput tidMe, tidTg, False
   If tidFg <> 0 AndAlso tidFg <> tidMe Then AttachThreadInput tidMe, tidFg, False
   Sleep 60                                     '等前台切换完成
End Sub

'-------------------------------------------------------------------------
' 供各符号页按钮调用的统一入口
'-------------------------------------------------------------------------
Sub InsertSymbol(ByVal s As CWSTR)         '左键：模拟键盘输入
   EnsureInputTarget
   InsertUnicodeText(s)
End Sub

Sub CopySymbol(ByVal s As CWSTR)           '右键：复制到剪贴板
   CopyUnicodeToClipboard(s)
End Sub
