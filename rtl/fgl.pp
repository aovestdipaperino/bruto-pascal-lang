{ Bruto Pascal runtime: FPC's fgl unit, generic lists and maps:
  TFPGList<T>, TFPGObjectList<T> (which frees its objects) and
  TFPGMap<TKey, TData> (sorted on request, searched with `<`). }
unit fgl;

{$mode objfpc}{$H+}

interface

uses SysUtils, Classes;

type
  generic TFPGCompareFunc<T> = function(const Item1, Item2: T): Integer;

  generic TFPGListEnumerator<T> = class
  private
    FItems: array of T;
    FCount: Integer;
    FPosition: Integer;
  public
    constructor Create(const AItems: array of T; ACount: Integer);
    function GetCurrent: T;
    function MoveNext: Boolean;
    property Current: T read GetCurrent;
  end;

  generic TFPGList<T> = class
  private
    FItems: array of T;
    FCount: Integer;
    function Get(Index: Integer): T;
    procedure Put(Index: Integer; const Item: T);
    function GetFirst: T;
    function GetLast: T;
    procedure SetCount(NewCount: Integer);
    function GetCapacity: Integer;
    procedure SetCapacity(NewCapacity: Integer);
    procedure CheckIndex(Index: Integer);
    procedure QuickSort(L, R: Integer; Compare: specialize TFPGCompareFunc<T>);
  public
    destructor Destroy; override;
    function Add(const Item: T): Integer;
    procedure Clear;
    procedure Delete(Index: Integer); virtual;
    procedure Exchange(Index1, Index2: Integer);
    function Extract(const Item: T): T;
    function IndexOf(const Item: T): Integer;
    procedure Insert(Index: Integer; const Item: T);
    procedure Move(CurIndex, NewIndex: Integer);
    function Remove(const Item: T): Integer;
    procedure Sort(Compare: specialize TFPGCompareFunc<T>);
    procedure Assign(Source: TFPGList);
    function GetEnumerator: specialize TFPGListEnumerator<T>;
    property Count: Integer read FCount write SetCount;
    property Capacity: Integer read GetCapacity write SetCapacity;
    property First: T read GetFirst;
    property Last: T read GetLast;
    property Items[Index: Integer]: T read Get write Put; default;
  end;

  generic TFPGObjectList<T> = class(specialize TFPGList<T>)
  private
    FFreeObjects: Boolean;
  public
    constructor Create(FreeObjects: Boolean = True);
    destructor Destroy; override;
    procedure Delete(Index: Integer); override;
    procedure Clear;
    property FreeObjects: Boolean read FFreeObjects write FFreeObjects;
  end;

  generic TFPGMap<TKey, TData> = class
  private
    FKeys: array of TKey;
    FData: array of TData;
    FCount: Integer;
    FSorted: Boolean;
    FDuplicates: TDuplicates;
    function GetKey(Index: Integer): TKey;
    function GetData(Index: Integer): TData;
    procedure PutData(Index: Integer; const AData: TData);
    function GetKeyData(const AKey: TKey): TData;
    procedure PutKeyData(const AKey: TKey; const AData: TData);
    procedure SetSorted(Value: Boolean);
    procedure CheckIndex(Index: Integer);
    procedure InsertAt(Index: Integer; const AKey: TKey; const AData: TData);
    function KeyCompare(const Key1, Key2: TKey): Integer;
  public
    function Add(const AKey: TKey; const AData: TData): Integer;
    function Find(const AKey: TKey; out Index: Integer): Boolean;
    function TryGetData(const AKey: TKey; out AData: TData): Boolean;
    function IndexOf(const AKey: TKey): Integer;
    function IndexOfData(const AData: TData): Integer;
    function Remove(const AKey: TKey): Integer;
    procedure Delete(Index: Integer);
    procedure Clear;
    procedure Sort;
    property Count: Integer read FCount;
    property Keys[Index: Integer]: TKey read GetKey;
    property Data[Index: Integer]: TData read GetData write PutData;
    property KeyData[const AKey: TKey]: TData read GetKeyData write PutKeyData; default;
    property Sorted: Boolean read FSorted write SetSorted;
    property Duplicates: TDuplicates read FDuplicates write FDuplicates;
  end;

implementation

function ListIndexError(Index: Integer): EListError;
begin
  ListIndexError := EListError.Create('List index (' + IntToStr(Index) + ') out of bounds');
end;

{ TFPGListEnumerator }

constructor TFPGListEnumerator.Create(const AItems: array of T; ACount: Integer);
var
  I: Integer;
begin
  inherited Create;
  SetLength(FItems, ACount);
  for I := 0 to ACount - 1 do
    FItems[I] := AItems[I];
  FCount := ACount;
  FPosition := -1;
end;

function TFPGListEnumerator.GetCurrent: T;
begin
  Result := FItems[FPosition];
end;

function TFPGListEnumerator.MoveNext: Boolean;
begin
  Inc(FPosition);
  Result := FPosition < FCount;
end;

{ TFPGList }

destructor TFPGList.Destroy;
begin
  Clear;
  inherited Destroy;
end;

procedure TFPGList.CheckIndex(Index: Integer);
begin
  if (Index < 0) or (Index >= FCount) then
    raise ListIndexError(Index);
end;

function TFPGList.Get(Index: Integer): T;
begin
  CheckIndex(Index);
  Result := FItems[Index];
end;

procedure TFPGList.Put(Index: Integer; const Item: T);
begin
  CheckIndex(Index);
  FItems[Index] := Item;
end;

function TFPGList.GetFirst: T;
begin
  Result := Get(0);
end;

function TFPGList.GetLast: T;
begin
  Result := Get(FCount - 1);
end;

function TFPGList.GetCapacity: Integer;
begin
  Result := Length(FItems);
end;

procedure TFPGList.SetCapacity(NewCapacity: Integer);
begin
  if NewCapacity < FCount then
    raise EListError.Create('List capacity (' + IntToStr(NewCapacity) + ') exceeded.');
  SetLength(FItems, NewCapacity);
end;

procedure TFPGList.SetCount(NewCount: Integer);
var
  I: Integer;
begin
  if NewCount < 0 then
    raise EListError.Create('List count (' + IntToStr(NewCount) + ') out of bounds.');
  if NewCount > Length(FItems) then
    SetLength(FItems, NewCount);
  for I := NewCount to FCount - 1 do
    FItems[I] := Default(T);
  FCount := NewCount;
end;

function TFPGList.Add(const Item: T): Integer;
begin
  if FCount = Length(FItems) then
    SetLength(FItems, FCount + FCount div 4 + 4);
  FItems[FCount] := Item;
  Result := FCount;
  Inc(FCount);
end;

procedure TFPGList.Clear;
begin
  SetLength(FItems, 0);
  FCount := 0;
end;

procedure TFPGList.Delete(Index: Integer);
var
  I: Integer;
begin
  CheckIndex(Index);
  for I := Index to FCount - 2 do
    FItems[I] := FItems[I + 1];
  Dec(FCount);
  FItems[FCount] := Default(T);
end;

procedure TFPGList.Exchange(Index1, Index2: Integer);
var
  Temp: T;
begin
  CheckIndex(Index1);
  CheckIndex(Index2);
  Temp := FItems[Index1];
  FItems[Index1] := FItems[Index2];
  FItems[Index2] := Temp;
end;

function TFPGList.Extract(const Item: T): T;
var
  I: Integer;
begin
  Result := Default(T);
  I := IndexOf(Item);
  if I >= 0 then
  begin
    Result := FItems[I];
    Delete(I);
  end;
end;

function TFPGList.IndexOf(const Item: T): Integer;
begin
  Result := 0;
  while (Result < FCount) and not (FItems[Result] = Item) do
    Inc(Result);
  if Result = FCount then
    Result := -1;
end;

procedure TFPGList.Insert(Index: Integer; const Item: T);
var
  I: Integer;
begin
  if (Index < 0) or (Index > FCount) then
    raise ListIndexError(Index);
  Add(Item);
  for I := FCount - 1 downto Index + 1 do
    FItems[I] := FItems[I - 1];
  FItems[Index] := Item;
end;

procedure TFPGList.Move(CurIndex, NewIndex: Integer);
var
  Temp: T;
begin
  if CurIndex = NewIndex then
    Exit;
  Temp := Get(CurIndex);
  Delete(CurIndex);
  Insert(NewIndex, Temp);
end;

function TFPGList.Remove(const Item: T): Integer;
begin
  Result := IndexOf(Item);
  if Result >= 0 then
    Delete(Result);
end;

procedure TFPGList.QuickSort(L, R: Integer; Compare: specialize TFPGCompareFunc<T>);
var
  I, J: Integer;
  P, Temp: T;
begin
  repeat
    I := L;
    J := R;
    P := FItems[(L + R) div 2];
    repeat
      while Compare(P, FItems[I]) > 0 do
        Inc(I);
      while Compare(P, FItems[J]) < 0 do
        Dec(J);
      if I <= J then
      begin
        Temp := FItems[I];
        FItems[I] := FItems[J];
        FItems[J] := Temp;
        Inc(I);
        Dec(J);
      end;
    until I > J;
    if L < J then
      QuickSort(L, J, Compare);
    L := I;
  until I >= R;
end;

procedure TFPGList.Sort(Compare: specialize TFPGCompareFunc<T>);
begin
  if FCount > 1 then
    QuickSort(0, FCount - 1, Compare);
end;

procedure TFPGList.Assign(Source: TFPGList);
var
  I: Integer;
begin
  Clear;
  for I := 0 to Source.Count - 1 do
    Add(Source[I]);
end;

function TFPGList.GetEnumerator: specialize TFPGListEnumerator<T>;
begin
  Result := specialize TFPGListEnumerator<T>.Create(FItems, FCount);
end;

{ TFPGObjectList }

constructor TFPGObjectList.Create(FreeObjects: Boolean);
begin
  inherited Create;
  FFreeObjects := FreeObjects;
end;

destructor TFPGObjectList.Destroy;
begin
  Clear;
  inherited Destroy;
end;

procedure TFPGObjectList.Delete(Index: Integer);
var
  Item: T;
begin
  CheckIndex(Index);
  Item := FItems[Index];
  inherited Delete(Index);
  if FFreeObjects then
    TObject(Item).Free;
end;

procedure TFPGObjectList.Clear;
var
  I: Integer;
begin
  if FFreeObjects then
    for I := 0 to FCount - 1 do
      TObject(FItems[I]).Free;
  inherited Clear;
end;

{ TFPGMap }

function TFPGMap.KeyCompare(const Key1, Key2: TKey): Integer;
begin
  if Key1 < Key2 then
    Result := -1
  else if Key2 < Key1 then
    Result := 1
  else
    Result := 0;
end;

procedure TFPGMap.CheckIndex(Index: Integer);
begin
  if (Index < 0) or (Index >= FCount) then
    raise ListIndexError(Index);
end;

function TFPGMap.GetKey(Index: Integer): TKey;
begin
  CheckIndex(Index);
  Result := FKeys[Index];
end;

function TFPGMap.GetData(Index: Integer): TData;
begin
  CheckIndex(Index);
  Result := FData[Index];
end;

procedure TFPGMap.PutData(Index: Integer; const AData: TData);
begin
  CheckIndex(Index);
  FData[Index] := AData;
end;

procedure TFPGMap.InsertAt(Index: Integer; const AKey: TKey; const AData: TData);
var
  I: Integer;
begin
  if FCount = Length(FKeys) then
  begin
    SetLength(FKeys, FCount + FCount div 4 + 4);
    SetLength(FData, Length(FKeys));
  end;
  for I := FCount downto Index + 1 do
  begin
    FKeys[I] := FKeys[I - 1];
    FData[I] := FData[I - 1];
  end;
  FKeys[Index] := AKey;
  FData[Index] := AData;
  Inc(FCount);
end;

function TFPGMap.Find(const AKey: TKey; out Index: Integer): Boolean;
var
  L, H, I, C: Integer;
begin
  Result := False;
  L := 0;
  H := FCount - 1;
  while L <= H do
  begin
    I := (L + H) div 2;
    C := KeyCompare(FKeys[I], AKey);
    if C < 0 then
      L := I + 1
    else
    begin
      H := I - 1;
      if C = 0 then
      begin
        Result := True;
        if FDuplicates <> dupAccept then
          L := I;
      end;
    end;
  end;
  Index := L;
end;

function TFPGMap.IndexOf(const AKey: TKey): Integer;
begin
  if FSorted then
  begin
    if not Find(AKey, Result) then
      Result := -1;
    Exit;
  end;
  Result := 0;
  while (Result < FCount) and (KeyCompare(FKeys[Result], AKey) <> 0) do
    Inc(Result);
  if Result = FCount then
    Result := -1;
end;

function TFPGMap.IndexOfData(const AData: TData): Integer;
begin
  Result := 0;
  while (Result < FCount) and not (FData[Result] = AData) do
    Inc(Result);
  if Result = FCount then
    Result := -1;
end;

function TFPGMap.Add(const AKey: TKey; const AData: TData): Integer;
begin
  if FSorted then
  begin
    if Find(AKey, Result) then
      case FDuplicates of
        dupIgnore: Exit;
        dupError: raise EListError.Create('Duplicate key');
      end;
  end
  else
    Result := FCount;
  InsertAt(Result, AKey, AData);
end;

function TFPGMap.TryGetData(const AKey: TKey; out AData: TData): Boolean;
var
  I: Integer;
begin
  I := IndexOf(AKey);
  Result := I >= 0;
  if Result then
    AData := FData[I]
  else
    AData := Default(TData);
end;

function TFPGMap.GetKeyData(const AKey: TKey): TData;
var
  I: Integer;
begin
  I := IndexOf(AKey);
  if I < 0 then
    raise EListError.Create('Key not found');
  Result := FData[I];
end;

procedure TFPGMap.PutKeyData(const AKey: TKey; const AData: TData);
var
  I: Integer;
begin
  I := IndexOf(AKey);
  if I >= 0 then
    FData[I] := AData
  else
    Add(AKey, AData);
end;

function TFPGMap.Remove(const AKey: TKey): Integer;
begin
  Result := IndexOf(AKey);
  if Result >= 0 then
    Delete(Result);
end;

procedure TFPGMap.Delete(Index: Integer);
var
  I: Integer;
begin
  CheckIndex(Index);
  for I := Index to FCount - 2 do
  begin
    FKeys[I] := FKeys[I + 1];
    FData[I] := FData[I + 1];
  end;
  Dec(FCount);
  FKeys[FCount] := Default(TKey);
  FData[FCount] := Default(TData);
end;

procedure TFPGMap.Clear;
begin
  SetLength(FKeys, 0);
  SetLength(FData, 0);
  FCount := 0;
end;

procedure TFPGMap.Sort;
var
  I, J: Integer;
  K: TKey;
  D: TData;
begin
  { Insertion sort: stable, and maps are usually small. }
  for I := 1 to FCount - 1 do
  begin
    K := FKeys[I];
    D := FData[I];
    J := I - 1;
    while (J >= 0) and (KeyCompare(FKeys[J], K) > 0) do
    begin
      FKeys[J + 1] := FKeys[J];
      FData[J + 1] := FData[J];
      Dec(J);
    end;
    FKeys[J + 1] := K;
    FData[J + 1] := D;
  end;
end;

procedure TFPGMap.SetSorted(Value: Boolean);
begin
  if Value and not FSorted then
    Sort;
  FSorted := Value;
end;

end.
