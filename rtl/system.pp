{ Bruto Pascal runtime: parts of FPC's System unit that are easiest to
  write in Pascal. Each declaration is used only when the program and its
  units don't declare one of the same name. }
unit BrutoSystem;

interface

type
  TFPCHeapStatus = record
    MaxHeapSize, MaxHeapUsed, CurrHeapSize, CurrHeapUsed, CurrHeapFree: PtrUInt;
  end;

{ Bruto doesn't track heap use: the figures are all zero. }
function GetFPCHeapStatus: TFPCHeapStatus;

implementation

function GetFPCHeapStatus: TFPCHeapStatus;
var
  Status: TFPCHeapStatus;
begin
  FillChar(Status, SizeOf(Status), 0);
  GetFPCHeapStatus := Status;
end;

end.
