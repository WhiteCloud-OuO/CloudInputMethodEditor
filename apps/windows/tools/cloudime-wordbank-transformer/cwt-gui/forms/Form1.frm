#VisualFreeBasic_Form#  Version=5.8.4
Locked=0

[Form]
Name=frmMain
ClassStyle=CS_DBLCLKS,CS_HREDRAW,CS_VREDRAW
ClassName=
WinStyle=WS_CAPTION,WS_CLIPCHILDREN,WS_CLIPSIBLINGS,WS_VISIBLE,WS_EX_CONTROLPARENT,WS_EX_LEFT,WS_EX_LTRREADING,WS_EX_RIGHTSCROLLBAR,WS_SYSMENU,WS_EX_TOPMOST,WS_POPUP
Style=3 - 常规窗口
Icon=cloudime.ico
Caption=词库转换工具
StartPosition=1 - 屏幕中心
WindowState=0 - 正常
Enabled=True
Repeat=False
Left=0
Top=0
Width=266
Height=186
TopMost=True
Child=False
MdiChild=False
TitleBar=True
SizeBox=False
SysMenu=True
MaximizeBox=False
MinimizeBox=False
Help=False
Hscroll=False
Vscroll=False
MinWidth=0
MinHeight=0
MaxWidth=0
MaxHeight=0
NoActivate=False
MousePass=False
TransPer=0
TransColor=SYS,25
Shadow=0 - 无阴影
BackColor=SYS,15
MousePointer=0 - 默认
Tag=
Tab=True
ToolTip=
ToolTipBalloon=False
AcceptFiles=True

[Label]
Name=Label1
Index=-1
Style=0 - 无边框
Caption=拖拽文件到此处。支持yaml, tsv, dat格式的词库。
Enabled=True
Visible=True
ForeColor=SYS,8
BackColor=SYS,25
Font=微软雅黑,9,0
TextAlign=0 - 左对齐
Prefix=True
Ellipsis=False
Left=39
Top=60
Width=182
Height=38
Layout=0 - 不锚定
MousePointer=0 - 默认
Tag=
ToolTip=
ToolTipBalloon=False


[AllCode]
'这是标准的工程模版，你也可做自己的模版。
'写好工程，复制全部文件到VFB软件文件夹里【template】里即可，子文件夹名为 VFB新建工程里显示的名称
'快去打造属于你自己的工程模版吧。

Sub frmMain_WM_DropFiles(hWndForm As hWnd ,HDROP As HDROP) '当用户将文件拖放到已启用接收的应用程序窗口中
   'hWndForm    当前窗口的句柄(WIN系统用来识别窗口的一个编号，如果多开本窗口，必须 Me.hWndForm = hWndForm 后才可以执行后续操作本窗口的代码)
   'hDrop       操作系统发来的拖拽数据，里面包括文件名等。
   ' !!!注意!!!! 需要窗口编辑里 "拖放"属性选择允许后，才可用。
   Dim u As Long = DragQueryFile(hDrop , -1 ,Null ,0) '获取文件个数
   If u Then
      Dim nFile As Wstring * (MAX_PATH + 1) ,re As Long ,i As Long
      For i = 0 To u -1
         re = DragQueryFileW(hDrop ,i ,@nFile ,MAX_PATH) '获取文件名，返回文件名的字符个数
         If re Then
            'nFile 包含路径的文件名，这里是宽字符版，要获取 A字符，使用 Dim nFile As zString 和 DragQueryFileA
            Dim tFile      As String = wStrToStr(nFile)
            Dim FileSuffix As String = GetFileExtensionBySplit(tFile)
            If FileSuffix = "dat" Or FileSuffix = "yaml" Or FileSuffix = "tsv" Or FileSuffix = "yml" Then
               Dim sFile As String = Left(tFile ,Len(tFile) - Len(FileSuffix)) & "db"
               'Debug.Print sFile
               Exec App.path & "cwt.exe" ,tFile & " " & sFile
            End If
            
         End If
      Next
   End If
   ExitProcess(0)
End Sub


Function GetFileExtensionBySplit(ByVal filePath As String) As String
   Dim parts() As String
   Dim ext     As String
   
   Dim ret As Long = vbSplit(filePath ,"." ,parts())
   
   ' UBound 返回数组的最大索引
   If UBound(parts) > 0 Then
      ext = parts(UBound(parts))
   Else
      ext = ""
   End If
   Debug.Print ext
   GetFileExtensionBySplit = ext
End Function