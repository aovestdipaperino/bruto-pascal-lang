{ Bruto Pascal runtime: the commonly used part of FPC's Math unit.

  The floating point exception mask is Bruto's: a division by zero, an
  invalid operation or an overflow stops the program with FPC's runtime
  error (or raises EZeroDivide, EInvalidOp or EOverflow with SysUtils)
  unless SetExceptionMask masks it, as FPC does on targets that trap. }
unit Math;

{$mode objfpc}{$H+}

interface

type
  Float = Double;
  PFloat = ^Float;

  TFPUException = (exInvalidOp, exDenormalized, exZeroDivide, exOverflow,
    exUnderflow, exPrecision);
  TFPUExceptionMask = set of TFPUException;

  TValueSign = -1..1;
  TValueRelationship = -1..1;
  TRoundToRange = -37..37;

const
  NegativeValue = -1;
  ZeroValue = 0;
  PositiveValue = 1;
  LessThanValue = -1;
  EqualsValue = 0;
  GreaterThanValue = 1;
  MinSingle = 1.5e-45;
  MaxSingle = 3.4e+38;
  MinDouble = 5.0e-324;
  MaxDouble = 1.7e+308;
  DZeroResolution = 1E-12;
  SZeroResolution = 1E-4;

var
  NaN, Infinity, NegInfinity: Double;

function GetExceptionMask: TFPUExceptionMask;
function SetExceptionMask(const Mask: TFPUExceptionMask): TFPUExceptionMask;

function Min(A, B: Integer): Integer; overload;
function Min(A, B: Int64): Int64; overload;
function Min(A, B: Double): Double; overload;
function Max(A, B: Integer): Integer; overload;
function Max(A, B: Int64): Int64; overload;
function Max(A, B: Double): Double; overload;

function InRange(const AValue, AMin, AMax: Int64): Boolean; overload;
function InRange(const AValue, AMin, AMax: Double): Boolean; overload;
function EnsureRange(const AValue, AMin, AMax: Int64): Int64; overload;
function EnsureRange(const AValue, AMin, AMax: Double): Double; overload;

function Sign(const AValue: Int64): TValueSign; overload;
function Sign(const AValue: Double): TValueSign; overload;
function CompareValue(const A, B: Int64): TValueRelationship; overload;
function CompareValue(const A, B: Double): TValueRelationship; overload;

function IsNan(const D: Double): Boolean;
function IsInfinite(const D: Double): Boolean;
function IsZero(const A: Double; Epsilon: Double = 0): Boolean;
function SameValue(const A, B: Double; Epsilon: Double = 0): Boolean;

function Floor(X: Float): Integer;
function Ceil(X: Float): Integer;
function Floor64(X: Float): Int64;
function Ceil64(X: Float): Int64;
function RoundTo(const AValue: Double; const Digits: TRoundToRange): Double;
function SimpleRoundTo(const AValue: Double; const Digits: TRoundToRange = -2): Double;

function IntPower(Base: Float; Exponent: LongInt): Float;
function Power(Base, Exponent: Float): Float;
function Log10(X: Float): Float;
function Log2(X: Float): Float;
function LogN(N, X: Float): Float;
function LnXP1(X: Float): Float;
function Hypot(X, Y: Float): Float;

function Tan(X: Float): Float;
function Cotan(X: Float): Float;
function ArcSin(X: Float): Float;
function ArcCos(X: Float): Float;
function ArcTan2(Y, X: Float): Float;
function Sinh(X: Float): Float;
function Cosh(X: Float): Float;
function Tanh(X: Float): Float;
function ArcSinh(X: Float): Float;
function ArcCosh(X: Float): Float;
function ArcTanh(X: Float): Float;
function DegToRad(Deg: Float): Float;
function RadToDeg(Rad: Float): Float;
function GradToRad(Grad: Float): Float;
function RadToGrad(Rad: Float): Float;
function CycleToRad(Cycle: Float): Float;
function RadToCycle(Rad: Float): Float;

function Sum(const Data: array of Double): Float;
function SumInt(const Data: array of Int64): Int64;
function Mean(const Data: array of Double): Float;
function MinValue(const Data: array of Double): Double; overload;
function MaxValue(const Data: array of Double): Double; overload;
function MinIntValue(const Data: array of Integer): Integer;
function MaxIntValue(const Data: array of Integer): Integer;
function SumOfSquares(const Data: array of Double): Float;
function Variance(const Data: array of Double): Float;
function StdDev(const Data: array of Double): Float;

function RandomRange(const AFrom, ATo: Integer): Integer;
function RandG(Mean, StdDev: Float): Float;
procedure DivMod(Dividend: LongInt; Divisor: Word; var Result, Remainder: Word); overload;
procedure DivMod(Dividend: LongInt; Divisor: LongInt; var Result, Remainder: LongInt); overload;

implementation

type
  PMathInt64 = ^Int64;

const
  { FPC's default: denormal, underflow and precision masked. }
  DefaultMaskBits = 50;
  MachEpsFloat = 2.2204460492503131e-16;
  MathLn2 = 0.693147180559945309417232121458176568;
  HugeDouble = 1e300;

function GetExceptionMask: TFPUExceptionMask;
var
  Bits: LongInt;
  E: TFPUException;
begin
  Result := [];
  Bits := BrutoFPUMaskFlip xor DefaultMaskBits;
  for E := Low(TFPUException) to High(TFPUException) do
    if (Bits and (1 shl Ord(E))) <> 0 then
      Include(Result, E);
end;

function SetExceptionMask(const Mask: TFPUExceptionMask): TFPUExceptionMask;
var
  Bits: LongInt;
  E: TFPUException;
begin
  Result := GetExceptionMask;
  Bits := 0;
  for E := Low(TFPUException) to High(TFPUException) do
    if E in Mask then
      Bits := Bits or (1 shl Ord(E));
  BrutoFPUMaskFlip := Bits xor DefaultMaskBits;
end;

function Min(A, B: Integer): Integer;
begin
  if A < B then Result := A else Result := B;
end;

function Min(A, B: Int64): Int64;
begin
  if A < B then Result := A else Result := B;
end;

function Min(A, B: Double): Double;
begin
  if A < B then Result := A else Result := B;
end;

function Max(A, B: Integer): Integer;
begin
  if A > B then Result := A else Result := B;
end;

function Max(A, B: Int64): Int64;
begin
  if A > B then Result := A else Result := B;
end;

function Max(A, B: Double): Double;
begin
  if A > B then Result := A else Result := B;
end;

function InRange(const AValue, AMin, AMax: Int64): Boolean;
begin
  Result := (AValue >= AMin) and (AValue <= AMax);
end;

function InRange(const AValue, AMin, AMax: Double): Boolean;
begin
  Result := (AValue >= AMin) and (AValue <= AMax);
end;

function EnsureRange(const AValue, AMin, AMax: Int64): Int64;
begin
  Result := AValue;
  if Result < AMin then
    Result := AMin
  else if Result > AMax then
    Result := AMax;
end;

function EnsureRange(const AValue, AMin, AMax: Double): Double;
begin
  Result := AValue;
  if Result < AMin then
    Result := AMin
  else if Result > AMax then
    Result := AMax;
end;

function Sign(const AValue: Int64): TValueSign;
begin
  if AValue < 0 then
    Result := -1
  else if AValue > 0 then
    Result := 1
  else
    Result := 0;
end;

function Sign(const AValue: Double): TValueSign;
begin
  if AValue < 0 then
    Result := -1
  else if AValue > 0 then
    Result := 1
  else
    Result := 0;
end;

function CompareValue(const A, B: Int64): TValueRelationship;
begin
  if A < B then
    Result := -1
  else if A > B then
    Result := 1
  else
    Result := 0;
end;

function CompareValue(const A, B: Double): TValueRelationship;
begin
  if A < B then
    Result := -1
  else if A > B then
    Result := 1
  else
    Result := 0;
end;

function IsNan(const D: Double): Boolean;
begin
  Result := D <> D;
end;

function IsInfinite(const D: Double): Boolean;
begin
  Result := (D = Infinity) or (D = NegInfinity);
end;

function IsZero(const A: Double; Epsilon: Double): Boolean;
begin
  if Epsilon = 0 then
    Epsilon := DZeroResolution;
  Result := Abs(A) <= Epsilon;
end;

function SameValue(const A, B: Double; Epsilon: Double): Boolean;
begin
  if Epsilon = 0 then
    Epsilon := Max(Min(Abs(A), Abs(B)) * DZeroResolution, DZeroResolution);
  if A > B then
    Result := (A - B) <= Epsilon
  else
    Result := (B - A) <= Epsilon;
end;

function Floor(X: Float): Integer;
begin
  Result := Trunc(X);
  if Frac(X) < 0 then
    Result := Result - 1;
end;

function Ceil(X: Float): Integer;
begin
  Result := Trunc(X);
  if Frac(X) > 0 then
    Result := Result + 1;
end;

function Floor64(X: Float): Int64;
begin
  Result := Trunc(X);
  if Frac(X) < 0 then
    Result := Result - 1;
end;

function Ceil64(X: Float): Int64;
begin
  Result := Trunc(X);
  if Frac(X) > 0 then
    Result := Result + 1;
end;

function RoundTo(const AValue: Double; const Digits: TRoundToRange): Double;
var
  RV, Q: Double;
begin
  RV := IntPower(10, Digits);
  Q := AValue / RV;
  { From 2^52 on, every Double is an integer. }
  if IsNan(Q) or (Abs(Q) >= 4503599627370496.0) then
    Result := AValue
  else
    Result := Round(Q) * RV;
end;

function SimpleRoundTo(const AValue: Double; const Digits: TRoundToRange): Double;
var
  RV: Double;
begin
  RV := IntPower(10, -Digits);
  if AValue < 0 then
    Result := Int((AValue * RV) - 0.5) / RV
  else
    Result := Int((AValue * RV) + 0.5) / RV;
end;

function IntPower(Base: Float; Exponent: LongInt): Float;
var
  I: LongInt;
begin
  if (Base = 0.0) and (Exponent = 0) then
    Result := 1
  else
  begin
    I := Abs(Exponent);
    Result := 1.0;
    while I > 0 do
    begin
      while (I and 1) = 0 do
      begin
        I := I shr 1;
        Base := Sqr(Base);
      end;
      I := I - 1;
      Result := Result * Base;
    end;
    if Exponent < 0 then
      Result := 1.0 / Result;
  end;
end;

function Power(Base, Exponent: Float): Float;
begin
  if Exponent = 0.0 then
    Result := 1.0
  else if (Base = 0.0) and (Exponent > 0.0) then
    Result := 0.0
  else if (Frac(Exponent) = 0.0) and (Abs(Exponent) <= MaxInt) then
    Result := IntPower(Base, Trunc(Exponent))
  else
    Result := Exp(Exponent * Ln(Base));
end;

function Log10(X: Float): Float;
begin
  Result := Ln(X) * 0.43429448190325182765;  { 1/ln(10) }
end;

function Log2(X: Float): Float;
begin
  Result := Ln(X) * 1.4426950408889634079;  { 1/ln(2) }
end;

function LogN(N, X: Float): Float;
begin
  Result := Ln(X) / Ln(N);
end;

function LnXP1(X: Float): Float;
var
  Y: Float;
begin
  if X >= 4.0 then
    Result := Ln(1.0 + X)
  else
  begin
    Y := 1.0 + X;
    if Y = 1.0 then
      Result := X
    else
    begin
      Result := Ln(Y);
      if Y > 0.0 then
        Result := Result + (X - (Y - 1.0)) / Y;
    end;
  end;
end;

function Hypot(X, Y: Float): Float;
begin
  X := Abs(X);
  Y := Abs(Y);
  if IsInfinite(X) or IsInfinite(Y) then
    Exit(Infinity);
  if IsNan(X) or IsNan(Y) then
    Exit(NaN);
  if X > Y then
    Result := X * Sqrt(1.0 + Sqr(Y / X))
  else if X > 0.0 then
    Result := Y * Sqrt(1.0 + Sqr(X / Y))
  else
    Result := Y;
end;

function Tan(X: Float): Float;
begin
  Result := Sin(X) / Cos(X);
end;

function Cotan(X: Float): Float;
begin
  Result := Cos(X) / Sin(X);
end;

function ArcSin(X: Float): Float;
begin
  Result := ArcTan2(X, Sqrt((1.0 - X) * (1.0 + X)));
end;

function ArcCos(X: Float): Float;
begin
  Result := ArcTan2(Sqrt((1.0 - X) * (1.0 + X)), X);
end;

function ArcTan2(Y, X: Float): Float;
begin
  if X = 0 then
  begin
    if Y = 0 then
      Result := 0.0
    else if Y > 0 then
      Result := Pi / 2
    else
      Result := -Pi / 2;
  end
  else
  begin
    Result := ArcTan(Y / X);
    if X < 0 then
      if Y < 0 then
        Result := Result - Pi
      else
        Result := Result + Pi;
  end;
end;

function Sinh(X: Float): Float;
var
  Temp: Float;
begin
  if X > 7.10475860073943942030E+0002 then
    Exit(HugeDouble * HugeDouble);
  if X < -7.10475860073943942030E+0002 then
    Exit(-(HugeDouble * HugeDouble));
  Temp := Exp(X);
  if Temp = 1 then
    Exit(X);
  Result := 0.5 * (Temp - 1.0 / Temp);
end;

function Cosh(X: Float): Float;
var
  Temp: Float;
begin
  if (X > 7.10475860073943942030E+0002) or (X < -7.10475860073943942030E+0002) then
    Exit(HugeDouble * HugeDouble);
  Temp := Exp(X);
  Result := 0.5 * (Temp + 1.0 / Temp);
end;

function Tanh(X: Float): Float;
var
  Tmp: Float;
begin
  if Abs(X) > 20 then
    Exit(Sign(X));
  if X < 0 then
  begin
    Tmp := Exp(2 * X);
    if Tmp = 1 then
      Exit(X);
    Result := (Tmp - 1) / (1 + Tmp);
  end
  else
  begin
    Tmp := Exp(-2 * X);
    if Tmp = 1 then
      Exit(X);
    Result := (1 - Tmp) / (1 + Tmp);
  end;
end;

function ArcSinh(X: Float): Float;
var
  Z: Float;
begin
  if IsNan(X) then
    Exit(X);
  Z := Abs(X);
  if Z > 1 / Sqrt(MachEpsFloat) then
    Z := Ln(Z) + MathLn2
  else if Z > 0.5 then
    Z := Ln(2 * Z + 1 / (Z + Sqrt(1 + Z * Z)))
  else
    Z := LnXP1(Z + Z * Z / (1 + Sqrt(1 + Z * Z)));
  { The sign of X, -0.0 included. }
  if (X < 0) or ((X = 0) and (PMathInt64(@X)^ < 0)) then
    Z := -Z;
  Result := Z;
end;

function ArcCosh(X: Float): Float;
var
  T: Float;
begin
  if IsNan(X) then
    Result := X
  else if X > 1 / Sqrt(MachEpsFloat) then
    Result := Ln(X) + MathLn2
  else if X < 2.0 then
  begin
    T := X - 1.0;
    Result := LnXP1(T + Sqrt(T * (T + 2.0)));
  end
  else
    Result := Ln(X + Sqrt((X - 1.0) * (X + 1.0)));
end;

function ArcTanh(X: Float): Float;
begin
  Result := (LnXP1(X) - LnXP1(-X)) * 0.5;
end;

function DegToRad(Deg: Float): Float;
begin
  Result := Deg * (Pi / 180.0);
end;

function RadToDeg(Rad: Float): Float;
begin
  Result := Rad * (180.0 / Pi);
end;

function GradToRad(Grad: Float): Float;
begin
  Result := Grad * (Pi / 200.0);
end;

function RadToGrad(Rad: Float): Float;
begin
  Result := Rad * (200.0 / Pi);
end;

function CycleToRad(Cycle: Float): Float;
begin
  Result := (2 * Pi) * Cycle;
end;

function RadToCycle(Rad: Float): Float;
begin
  Result := Rad * (1 / (2 * Pi));
end;

function Sum(const Data: array of Double): Float;
var
  I: Integer;
begin
  Result := 0.0;
  for I := 0 to High(Data) do
    Result := Result + Data[I];
end;

function SumInt(const Data: array of Int64): Int64;
var
  I: Integer;
begin
  Result := 0;
  for I := 0 to High(Data) do
    Result := Result + Data[I];
end;

function Mean(const Data: array of Double): Float;
begin
  Result := Sum(Data) / Length(Data);
end;

function MinValue(const Data: array of Double): Double;
var
  I: Integer;
begin
  Result := Data[0];
  for I := 1 to High(Data) do
    if Data[I] < Result then
      Result := Data[I];
end;

function MaxValue(const Data: array of Double): Double;
var
  I: Integer;
begin
  Result := Data[0];
  for I := 1 to High(Data) do
    if Data[I] > Result then
      Result := Data[I];
end;

function MinIntValue(const Data: array of Integer): Integer;
var
  I: Integer;
begin
  Result := Data[0];
  for I := 1 to High(Data) do
    if Data[I] < Result then
      Result := Data[I];
end;

function MaxIntValue(const Data: array of Integer): Integer;
var
  I: Integer;
begin
  Result := Data[0];
  for I := 1 to High(Data) do
    if Data[I] > Result then
      Result := Data[I];
end;

function SumOfSquares(const Data: array of Double): Float;
var
  I: Integer;
begin
  Result := 0.0;
  for I := 0 to High(Data) do
    Result := Result + Sqr(Data[I]);
end;

function Variance(const Data: array of Double): Float;
var
  I: Integer;
  M: Float;
begin
  Result := 0.0;
  if Length(Data) < 2 then
    Exit;
  M := Mean(Data);
  for I := 0 to High(Data) do
    Result := Result + Sqr(Data[I] - M);
  Result := Result / (Length(Data) - 1);
end;

function StdDev(const Data: array of Double): Float;
begin
  Result := Sqrt(Variance(Data));
end;

function RandomRange(const AFrom, ATo: Integer): Integer;
begin
  Result := Random(Abs(AFrom - ATo)) + Min(ATo, AFrom);
end;

function RandG(Mean, StdDev: Float): Float;
var
  U1, S2: Float;
begin
  { The polar Box-Muller method, as FPC's. }
  repeat
    U1 := 2 * Random - 1;
    S2 := Sqr(U1) + Sqr(2 * Random - 1);
  until (S2 < 1) and (S2 <> 0);
  Result := Sqrt(-2 * Ln(S2) / S2) * U1 * StdDev + Mean;
end;

procedure DivMod(Dividend: LongInt; Divisor: Word; var Result, Remainder: Word);
begin
  Result := Dividend div Divisor;
  Remainder := Dividend mod Divisor;
end;

procedure DivMod(Dividend: LongInt; Divisor: LongInt; var Result, Remainder: LongInt);
begin
  Result := Dividend div Divisor;
  Remainder := Dividend mod Divisor;
end;

initialization
  PMathInt64(@Infinity)^ := $7FF0000000000000;
  PMathInt64(@NegInfinity)^ := Int64($FFF0000000000000);
  PMathInt64(@NaN)^ := Int64($FFF8000000000000);
end.
